//! Reading SSH key material for the keychain (CRED-02, CRED-03): prove that
//! a private key can be unlocked and describe it by its public half. Nothing
//! here connects to a host, and no message contains key material.

use crate::core_err;
use anyhow::Result;
use russh::keys::{decode_secret_key, Certificate, PrivateKey, PublicKey};
use senvakara_core::ErrorCode;

/// The public description of a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// Algorithm as OpenSSH names it, for example `ssh-ed25519`.
    pub algorithm: String,
    /// OpenSSH public key line.
    pub public_key: String,
}

fn invalid(message: &str) -> anyhow::Error {
    core_err(ErrorCode::InvalidInput, message)
}

fn info(key: &PublicKey) -> Result<KeyInfo> {
    Ok(KeyInfo {
        algorithm: key.algorithm().as_str().to_string(),
        public_key: key
            .to_openssh()
            .map_err(|_| invalid("the public key cannot be written out"))?,
    })
}

fn unlock(private_key: &str, passphrase: Option<&str>) -> Result<PrivateKey> {
    match decode_secret_key(private_key, passphrase) {
        Ok(key) => Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => Err(invalid(match passphrase {
            None => "this private key is encrypted: enter its passphrase",
            Some(_) => "the passphrase does not unlock this private key",
        })),
        Err(_) => Err(invalid(match passphrase {
            None => "this is not a private key Senvakara can read (OpenSSH, PEM or PuTTY format)",
            Some(_) => "the private key cannot be read: check the passphrase and the key text",
        })),
    }
}

/// Decrypt `private_key` (OpenSSH, PEM or PuTTY text) and describe it.
/// Blocking: decrypting runs a slow key derivation, keep it off async workers.
///
/// Fails with `InvalidInput` when the text is not a key, is encrypted and no
/// passphrase was given, or the passphrase is wrong.
pub fn inspect_private_key(private_key: &str, passphrase: Option<&str>) -> Result<KeyInfo> {
    info(unlock(private_key, passphrase)?.public_key())
}

/// Whether private key text is protected by a passphrase, told from its headers without
/// unlocking it or deriving any key: an OpenSSH key with a cipher other than `none`, legacy PEM
/// with `Proc-Type: 4,ENCRYPTED`, PKCS#8 `BEGIN ENCRYPTED PRIVATE KEY`, or a PuTTY key whose
/// `Encryption:` is not `none`. Text that is no key of those kinds is not "encrypted".
pub fn is_encrypted_private_key(text: &str) -> bool {
    let text = text.trim();
    if text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
        return true;
    }
    if text.contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
        return PrivateKey::from_openssh(text).is_ok_and(|key| key.is_encrypted());
    }
    if text.starts_with("PuTTY-User-Key-File") {
        return text.lines().any(|line| {
            line.strip_prefix("Encryption:")
                .is_some_and(|cipher| !cipher.trim().eq_ignore_ascii_case("none"))
        });
    }
    // Legacy PEM: the header sits inside the block, before the body.
    text.contains("PRIVATE KEY-----")
        && text.lines().any(|line| {
            line.trim()
                .strip_prefix("Proc-Type:")
                .is_some_and(|value| value.contains("ENCRYPTED"))
        })
}

/// Parse an OpenSSH public key line (`ssh-ed25519 AAAA... comment`).
pub fn inspect_public_key(line: &str) -> Result<KeyInfo> {
    let key = PublicKey::from_openssh(line.trim())
        .map_err(|_| invalid("this is not an OpenSSH public key line"))?;
    info(&key)
}

/// Whether two OpenSSH public key lines name the same key (comments aside).
pub fn same_public_key(a: &str, b: &str) -> bool {
    match (
        PublicKey::from_openssh(a.trim()),
        PublicKey::from_openssh(b.trim()),
    ) {
        (Ok(a), Ok(b)) => a.key_data() == b.key_data(),
        _ => false,
    }
}

/// Check that `certificate` is an OpenSSH certificate line issued for the
/// key in `public_key`. Its signature and validity are the server's to judge.
pub fn check_certificate(certificate: &str, public_key: &str) -> Result<()> {
    let certificate = Certificate::from_openssh(certificate.trim())
        .map_err(|_| invalid("this is not an OpenSSH certificate line (the -cert.pub file)"))?;
    let key = PublicKey::from_openssh(public_key.trim())
        .map_err(|_| invalid("this is not an OpenSSH public key line"))?;
    if certificate.public_key() != key.key_data() {
        return Err(invalid("the certificate was issued for a different key"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::LineEnding;
    use russh::keys::Algorithm;

    fn key() -> PrivateKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
    }

    fn message(error: anyhow::Error) -> String {
        let core = error
            .downcast_ref::<senvakara_core::CoreError>()
            .expect("CoreError");
        assert_eq!(core.code, ErrorCode::InvalidInput);
        core.message.clone()
    }

    #[test]
    fn a_private_key_is_described_by_its_public_half() {
        let key = key();
        let text = key.to_openssh(LineEnding::LF).unwrap();
        let info = inspect_private_key(&text, None).unwrap();
        assert_eq!(info.algorithm, "ssh-ed25519");
        assert!(same_public_key(
            &info.public_key,
            &key.public_key().to_openssh().unwrap()
        ));
        assert_eq!(inspect_public_key(&info.public_key).unwrap(), info);
    }

    #[test]
    fn an_encrypted_key_asks_for_the_passphrase_and_refuses_a_wrong_one() {
        let locked = key().encrypt(&mut rand::rng(), "open sesame").unwrap();
        let text = locked.to_openssh(LineEnding::LF).unwrap();
        assert!(message(inspect_private_key(&text, None).unwrap_err()).contains("enter its"));
        let wrong = message(inspect_private_key(&text, Some("guess")).unwrap_err());
        assert!(!wrong.contains("guess"), "{wrong}");
        assert!(inspect_private_key(&text, Some("open sesame")).is_ok());
    }

    #[test]
    fn an_openssh_key_is_encrypted_only_when_it_has_a_cipher() {
        let plain = key();
        let plain_text = plain.to_openssh(LineEnding::LF).unwrap();
        assert!(!is_encrypted_private_key(&plain_text));
        let locked = plain.encrypt(&mut rand::rng(), "open sesame").unwrap();
        for ending in [LineEnding::LF, LineEnding::CRLF] {
            let text = locked.to_openssh(ending).unwrap();
            assert!(is_encrypted_private_key(&text));
            assert!(is_encrypted_private_key(&format!(
                "\n  {}\n\n",
                text.as_str()
            )));
        }
    }

    #[test]
    fn other_encrypted_formats_are_told_from_their_headers() {
        let pem_locked = "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n\
                          DEK-Info: AES-128-CBC,00112233445566778899AABBCCDDEEFF\n\nAAAA\n\
                          -----END RSA PRIVATE KEY-----\n";
        let pem_plain = "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n";
        let pkcs8_locked =
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
        let pkcs8_plain = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
        let putty_locked = "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: aes256-cbc\n";
        let putty_plain = "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n";
        assert!(is_encrypted_private_key(pem_locked));
        assert!(is_encrypted_private_key(pkcs8_locked));
        assert!(is_encrypted_private_key(putty_locked));
        assert!(!is_encrypted_private_key(pem_plain));
        assert!(!is_encrypted_private_key(pkcs8_plain));
        assert!(!is_encrypted_private_key(putty_plain));
    }

    #[test]
    fn text_that_is_not_a_private_key_is_not_called_encrypted() {
        assert!(!is_encrypted_private_key(""));
        assert!(!is_encrypted_private_key("hello there"));
        assert!(!is_encrypted_private_key("Proc-Type: 4,ENCRYPTED"));
        let public = key().public_key().to_openssh().unwrap();
        assert!(!is_encrypted_private_key(&public));
        let cut = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n";
        assert!(!is_encrypted_private_key(cut));
    }

    #[test]
    fn text_that_is_not_a_key_is_refused_without_being_echoed() {
        let said = message(inspect_private_key("hello there", None).unwrap_err());
        assert!(!said.contains("hello"), "{said}");
        assert!(inspect_public_key("ssh-ed25519 not-base64").is_err());
        assert!(!same_public_key("garbage", "garbage"));
    }
}
