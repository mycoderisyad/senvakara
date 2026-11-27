//! Secret settings (ADR-033). The graph keeps only a Credential Manager
//! reference; the app resolves it right before a run. The value never
//! enters the script file, argv, the database or a log: it travels on the
//! script's stdin, after the sudo password line, between two marker lines
//! that carry a fresh nonce, base64-encoded so any byte but NUL survives.

use crate::params::ParamValue;
use std::collections::BTreeMap;

/// `(setting name, reference)` of every secret setting.
pub fn secret_refs(values: &BTreeMap<String, ParamValue>) -> Vec<(String, String)> {
    values
        .iter()
        .filter_map(|(name, v)| match v {
            ParamValue::SecretRef(r) => Some((name.clone(), r.clone())),
            _ => None,
        })
        .collect()
}

/// The settings that may be written into a script file.
pub fn without_secrets(values: &BTreeMap<String, ParamValue>) -> BTreeMap<String, ParamValue> {
    values
        .iter()
        .filter(|(_, v)| !matches!(v, ParamValue::SecretRef(_)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The stdin block the launcher reads: `SV-SECRETS <nonce>`, one
/// `SV_<NAME> <base64>` line per secret, `SV-SECRETS-END <nonce>`.
///
/// Fails when the nonce is not a plain token or a value holds a NUL byte
/// (bash variables cannot).
pub fn secret_block(secrets: &[(String, String)], nonce: &str) -> Result<String, String> {
    let token_ok = !nonce.is_empty()
        && nonce.len() <= 64
        && nonce.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !token_ok {
        return Err("bad secret marker".into());
    }
    let mut out = format!("SV-SECRETS {nonce}\n");
    for (name, value) in secrets {
        if value.contains('\0') {
            return Err(format!("the secret for `{name}` holds a NUL byte"));
        }
        let ident_ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ident_ok {
            return Err(format!("bad secret name `{name}`"));
        }
        out.push_str(&format!(
            "SV_{} {}\n",
            name.to_ascii_uppercase(),
            base64(value.as_bytes())
        ));
    }
    out.push_str(&format!("SV-SECRETS-END {nonce}\n"));
    Ok(out)
}

/// Standard base64 with padding, as `base64 -d` from coreutils reads it.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn the_block_carries_hostile_values_as_plain_tokens() {
        let secrets = vec![
            (
                "db_password".to_string(),
                "'; rm -rf / $(id)\nSV-SECRETS-END x".to_string(),
            ),
            ("token".to_string(), String::new()),
        ];
        let block = secret_block(&secrets, "n-1").unwrap();
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "SV-SECRETS n-1");
        assert!(lines[1].starts_with("SV_DB_PASSWORD "));
        assert!(!block.contains("rm -rf") && !block.contains("$(id)"));
        assert_eq!(lines[2], "SV_TOKEN ");
        assert_eq!(lines[3], "SV-SECRETS-END n-1");
        assert!(secret_block(&[("a".into(), "x\0y".into())], "n").is_err());
        assert!(secret_block(&[("A b".into(), "x".into())], "n").is_err());
        assert!(secret_block(&[], "bad nonce").is_err());
    }

    #[test]
    fn secrets_are_split_from_what_a_file_may_hold() {
        let mut values = BTreeMap::new();
        values.insert("port".to_string(), ParamValue::Int(80));
        values.insert(
            "token".to_string(),
            ParamValue::SecretRef("secret-9".into()),
        );
        assert_eq!(
            secret_refs(&values),
            vec![("token".to_string(), "secret-9".to_string())]
        );
        let public = without_secrets(&values);
        assert_eq!(public.len(), 1);
        assert!(public.contains_key("port"));
    }
}
