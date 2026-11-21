//! Best-effort redaction for log lines and error text (NFR-07,
//! docs/SECURITY.md §3.7). It is a safety net, not a licence to log secrets:
//! callers must still avoid passing secret values to loggers.

const KEYS: &[&str] = &[
    "password",
    "passphrase",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
];

/// Replace PEM private-key blocks and `key=value` / `key: value` pairs whose
/// key looks secret with `[REDACTED]`.
pub fn redact(input: &str) -> String {
    redact_pairs(&redact_pem(input))
}

fn redact_pem(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("-----BEGIN ") {
        let header_end = match rest[start..]
            .find("-----\n")
            .or_else(|| rest[start + 11..].find("-----").map(|i| i + 11))
        {
            Some(i) => start + i,
            None => break,
        };
        if !rest[start..header_end].contains("PRIVATE KEY") {
            out.push_str(&rest[..header_end]);
            rest = &rest[header_end..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str("[REDACTED PRIVATE KEY]");
        rest = match rest[header_end..].find("-----END ") {
            Some(i) => {
                let after = header_end + i + 9;
                match rest[after..].find("-----") {
                    Some(j) => &rest[after + j + 5..],
                    None => "",
                }
            }
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn redact_pairs(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let hit = KEYS.iter().find(|k| {
            lower[i..].starts_with(*k) && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
        });
        if let Some(k) = hit {
            let mut j = i + k.len();
            // Allow quotes and spaces around the separator: key="v", key : v
            while j < input.len() && matches!(bytes[j], b'"' | b'\'' | b' ') {
                j += 1;
            }
            if j < input.len() && matches!(bytes[j], b'=' | b':') {
                j += 1;
                while j < input.len() && bytes[j] == b' ' {
                    j += 1;
                }
                // A quoted value runs to its closing quote, spaces included.
                let quote =
                    (j < input.len() && matches!(bytes[j], b'"' | b'\'')).then(|| bytes[j] as char);
                if quote.is_some() {
                    j += 1;
                }
                let value_end = match quote {
                    Some(q) => input[j..].find(q).map(|n| j + n).unwrap_or(input.len()),
                    None => input[j..]
                        .find(|c: char| c.is_whitespace() || matches!(c, '&' | ';' | ','))
                        .map(|n| j + n)
                        .unwrap_or(input.len()),
                };
                if value_end > j {
                    out.push_str(&input[i..j]);
                    out.push_str("[REDACTED]");
                    i = value_end;
                    continue;
                }
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redacts_key_value_pairs() {
        assert_eq!(
            redact("user=dev password=hunter2 port=22"),
            "user=dev password=[REDACTED] port=22"
        );
        assert_eq!(
            redact(r#"{"passphrase": "abc def"}"#),
            r#"{"passphrase": "[REDACTED]"}"#
        );
        assert_eq!(
            redact("password='a b c' next"),
            "password='[REDACTED]' next"
        );
        assert_eq!(redact("Token: xyz;next"), "Token: [REDACTED];next");
        assert_eq!(redact("url?api_key=k1&x=1"), "url?api_key=[REDACTED]&x=1");
    }

    #[test]
    fn leaves_harmless_text_alone() {
        let s = "tokenizer ok; password field empty; secretary=bob";
        assert_eq!(redact(s), s);
        assert_eq!(
            redact("host key SHA256:abc accepted"),
            "host key SHA256:abc accepted"
        );
    }

    #[test]
    fn redacts_pem_private_keys_only() {
        let s = "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\nBBBB\n-----END OPENSSH PRIVATE KEY-----\nb";
        assert_eq!(redact(s), "a\n[REDACTED PRIVATE KEY]\nb");
        let pubkey = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----";
        assert_eq!(redact(pubkey), pubkey);
    }
}
