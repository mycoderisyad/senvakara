//! Checking what the key form holds, without touching the UI. The backend checks again, and
//! for real: it unlocks the key and matches the public key and certificate against it. These
//! checks only catch what can be told from the text, so the note lands under the right field.

use super::Mode;

/// The longest label a key may have (the storage layer's limit).
const MAX_LABEL_CHARS: usize = 64;

/// The fields that are not secret. Compared with how the form opened to tell whether the user
/// changed anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    pub label: String,
    pub public_key: String,
    pub certificate: String,
}

/// What is wrong with each field, in words. An empty string means the field is fine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub label: &'static str,
    pub private_key: &'static str,
    pub public_key: &'static str,
    pub certificate: &'static str,
}

impl Problems {
    pub fn any(&self) -> bool {
        [
            self.label,
            self.private_key,
            self.public_key,
            self.certificate,
        ]
        .iter()
        .any(|problem| !problem.is_empty())
    }
}

/// The algorithm an OpenSSH public key or certificate line starts with, when it has the shape
/// of one: a single line of an algorithm, the encoded key and an optional comment.
fn line_algorithm(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() || text.lines().count() != 1 {
        return None;
    }
    let mut words = text.split_whitespace();
    let algorithm = words.next()?;
    let encoded = words.next()?;
    let known = ["ssh-", "ecdsa-", "sk-", "rsa-"]
        .iter()
        .any(|start| algorithm.starts_with(start));
    (known && encoded.len() >= 16).then_some(algorithm)
}

fn label_problem(label: &str) -> &'static str {
    let label = label.trim();
    if label.is_empty() {
        "Give the key a label."
    } else if label.chars().count() > MAX_LABEL_CHARS {
        "A label can be at most 64 characters."
    } else {
        ""
    }
}

/// Why `text` cannot be a private key, or "" when it may be one.
pub fn private_key_problem(text: &str) -> &'static str {
    let text = text.trim();
    if text.is_empty() {
        "Paste the private key, or import it from a file."
    } else if line_algorithm(text).is_some() {
        "This is a public key. The private key is the file without .pub; it begins with -----BEGIN."
    } else if !(text.contains("PRIVATE KEY-----") || text.starts_with("PuTTY-User-Key-File")) {
        "This does not look like a private key. It begins with -----BEGIN OPENSSH PRIVATE KEY----- or a similar line."
    } else {
        ""
    }
}

fn public_key_problem(text: &str) -> &'static str {
    match line_algorithm(text) {
        _ if text.trim().is_empty() => "",
        Some(algorithm) if algorithm.contains("-cert-") => {
            "This is a certificate. It goes in the Certificate field."
        }
        Some(_) => "",
        None => "A public key is one line, such as ssh-ed25519 AAAA... name.",
    }
}

fn certificate_problem(text: &str, required: bool) -> &'static str {
    match line_algorithm(text) {
        _ if text.trim().is_empty() && required => {
            "Paste the certificate: the line in the -cert.pub file."
        }
        _ if text.trim().is_empty() => "",
        Some(algorithm) if algorithm.contains("-cert-") => "",
        Some(_) => {
            "This is a public key, not a certificate. A certificate comes from the -cert.pub file."
        }
        None => "A certificate is one line, such as ssh-ed25519-cert-v01@openssh.com AAAA...",
    }
}

/// Check a new entry.
pub fn check_new(mode: Mode, private_key: &str, kept: &Kept) -> Problems {
    let mut problems = Problems {
        label: label_problem(&kept.label),
        ..Problems::default()
    };
    if mode == Mode::Fido2 {
        problems.public_key = match line_algorithm(&kept.public_key) {
            _ if kept.public_key.trim().is_empty() => {
                "Paste the public key line of the security key (the .pub file)."
            }
            Some(algorithm) if algorithm.starts_with("sk-") && !algorithm.contains("-cert-") => "",
            Some(_) => "This is not a FIDO2 key: its type starts with sk-, such as sk-ssh-ed25519@openssh.com.",
            None => "A public key is one line, such as sk-ssh-ed25519@openssh.com AAAA... name.",
        };
        return problems;
    }
    problems.private_key = private_key_problem(private_key);
    problems.public_key = public_key_problem(&kept.public_key);
    problems.certificate = certificate_problem(&kept.certificate, mode == Mode::Certificate);
    problems
}

/// Check the changes to a saved key. `new_private` is the private key replacing the saved
/// one, when the user asked to replace it.
pub fn check_edit(fido: bool, new_private: Option<&str>, kept: &Kept) -> Problems {
    Problems {
        label: label_problem(&kept.label),
        private_key: match new_private {
            Some(text) if !fido => private_key_problem(text),
            _ => "",
        },
        public_key: "",
        certificate: certificate_problem(&kept.certificate, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIVATE: &str =
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    const PUBLIC: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleExampleExample alice@laptop";
    const CERTIFICATE: &str =
        "ssh-ed25519-cert-v01@openssh.com AAAAIHNzaC1lZDI1NTE5LWNlcnQtdjAxQG9wZW5zc2g alice";
    const SECURITY_KEY: &str =
        "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t alice@token";

    fn kept(public_key: &str, certificate: &str) -> Kept {
        Kept {
            label: "laptop".into(),
            public_key: public_key.into(),
            certificate: certificate.into(),
        }
    }

    #[test]
    fn a_pasted_private_key_with_a_label_is_enough() {
        let problems = check_new(Mode::Key, PRIVATE, &kept("", ""));
        assert!(!problems.any(), "{problems:?}");
        let full = check_new(Mode::Key, PRIVATE, &kept(PUBLIC, CERTIFICATE));
        assert!(!full.any(), "{full:?}");
        let putty = "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n";
        assert_eq!(private_key_problem(putty), "");
    }

    #[test]
    fn an_empty_form_asks_for_the_label_and_the_private_key() {
        let unnamed = Kept {
            label: "  ".into(),
            ..kept("", "")
        };
        let problems = check_new(Mode::Key, "", &unnamed);
        assert_eq!(problems.label, "Give the key a label.");
        assert!(problems.private_key.starts_with("Paste the private key"));
        assert_eq!((problems.public_key, problems.certificate), ("", ""));
        let long = Kept {
            label: "l".repeat(65),
            ..kept("", "")
        };
        assert_eq!(
            check_new(Mode::Key, PRIVATE, &long).label,
            "A label can be at most 64 characters."
        );
    }

    #[test]
    fn a_public_key_in_the_private_field_is_named_as_such() {
        assert!(private_key_problem(PUBLIC).starts_with("This is a public key"));
        assert!(private_key_problem("hello").starts_with("This does not look like a private key"));
    }

    #[test]
    fn each_line_field_takes_only_its_own_kind_of_line() {
        let swapped = check_new(Mode::Key, PRIVATE, &kept(CERTIFICATE, PUBLIC));
        assert!(swapped.public_key.starts_with("This is a certificate"));
        assert!(swapped.certificate.starts_with("This is a public key"));
        let broken = check_new(Mode::Key, PRIVATE, &kept("two\nlines", "nonsense"));
        assert!(broken.public_key.starts_with("A public key is one line"));
        assert!(broken.certificate.starts_with("A certificate is one line"));
    }

    #[test]
    fn a_key_added_as_a_certificate_needs_the_certificate() {
        let missing = check_new(Mode::Certificate, PRIVATE, &kept("", ""));
        assert!(missing.certificate.starts_with("Paste the certificate"));
        let given = check_new(Mode::Certificate, PRIVATE, &kept("", CERTIFICATE));
        assert!(!given.any(), "{given:?}");
    }

    #[test]
    fn a_fido2_entry_needs_only_a_label_and_an_sk_public_key() {
        let fine = check_new(Mode::Fido2, "", &kept(SECURITY_KEY, ""));
        assert!(!fine.any(), "{fine:?}");
        let empty = check_new(Mode::Fido2, "", &kept("", ""));
        assert!(empty.public_key.starts_with("Paste the public key line"));
        assert_eq!(empty.private_key, "");
        let plain = check_new(Mode::Fido2, "", &kept(PUBLIC, ""));
        assert!(plain.public_key.starts_with("This is not a FIDO2 key"));
    }

    #[test]
    fn editing_checks_the_label_the_certificate_and_a_replacement_key() {
        assert!(!check_edit(false, None, &kept(PUBLIC, "")).any());
        assert!(!check_edit(false, Some(PRIVATE), &kept(PUBLIC, CERTIFICATE)).any());
        let problems = check_edit(false, Some(""), &kept(PUBLIC, PUBLIC));
        assert!(problems.private_key.starts_with("Paste the private key"));
        assert!(problems.certificate.starts_with("This is a public key"));
        // A security key has no private text to replace.
        assert!(!check_edit(true, Some(""), &kept(SECURITY_KEY, "")).any());
        let unnamed = Kept {
            label: String::new(),
            ..kept(PUBLIC, "")
        };
        assert_eq!(
            check_edit(false, None, &unnamed).label,
            "Give the key a label."
        );
    }
}
