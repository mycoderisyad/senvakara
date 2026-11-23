//! The keychain against a throw-away database and the real Windows
//! Credential Manager (every entry written is removed again).

#![cfg(windows)]

use super::*;
use russh::keys::ssh_key::public::{Ed25519PublicKey, KeyData, SkEd25519};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use senvakara_storage::{HostUpsert, Protocol, SshAuth};
use std::path::PathBuf;

struct Fixture {
    dir: PathBuf,
    state: AppState,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Whatever a failed test left behind must not stay in the OS store.
        if let Ok(keys) = self.state.db.lock().unwrap().list_keys() {
            if let Some(store) = &self.state.secrets {
                for key in keys {
                    let _ = key.private_ref.map(|r| store.delete_large(&r));
                    let _ = key.passphrase_ref.map(|r| store.delete(&r));
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!("svk-keychain-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = AppState::open(dir.join("test.db")).unwrap();
    assert!(state.secrets.is_some(), "the credential store is needed");
    Fixture { dir, state }
}

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn text(key: &PrivateKey) -> String {
    key.to_openssh(LineEnding::LF).unwrap().to_string()
}

fn new_key(label: &str, key: &PrivateKey) -> KeyCreate {
    KeyCreate {
        label: label.into(),
        private_key: text(key),
        ..Default::default()
    }
}

fn record(state: &AppState, id: &str) -> KeyRecord {
    state.db.lock().unwrap().get_key(id).unwrap()
}

fn secret(state: &AppState, reference: &str) -> Option<String> {
    state
        .secrets
        .as_ref()
        .unwrap()
        .get_large(reference)
        .unwrap()
}

#[test]
fn a_new_key_keeps_its_secrets_in_the_os_store_and_shows_only_public_parts() {
    let f = fixture();
    let key = key();
    let view = keychain_create(&f.state, new_key("  laptop  ", &key)).unwrap();
    assert_eq!(view.label, "laptop");
    assert_eq!(view.algorithm, "ssh-ed25519");
    assert!(same_public_key(
        &view.public_key,
        &key.public_key().to_openssh().unwrap()
    ));
    assert!(view.has_private_key && !view.has_passphrase);
    assert!(view.used_by.is_empty());
    assert_eq!(keychain_list(&f.state).unwrap().len(), 1);

    let row = record(&f.state, &view.id);
    let private_ref = row.private_ref.unwrap();
    assert_eq!(secret(&f.state, &private_ref), Some(text(&key)));
    // The view the UI gets holds no key text.
    let shown = serde_json::to_string(&view).unwrap();
    assert!(!shown.contains("PRIVATE KEY") && !shown.contains(&private_ref));

    keychain_delete(&f.state, &view.id).unwrap();
    assert_eq!(secret(&f.state, &private_ref), None);
    assert!(keychain_list(&f.state).unwrap().is_empty());
}

#[test]
fn an_encrypted_key_is_only_added_with_the_passphrase_that_unlocks_it() {
    let f = fixture();
    let locked = key().encrypt(&mut rand::rng(), "open sesame").unwrap();
    let mut input = new_key("locked", &locked);

    let error = keychain_create(&f.state, input.clone()).err().unwrap();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("passphrase"), "{error}");
    input.passphrase = Some("wrong guess".into());
    let error = keychain_create(&f.state, input.clone()).err().unwrap();
    assert!(!error.message.contains("wrong guess"), "{error}");
    assert!(keychain_list(&f.state).unwrap().is_empty());

    input.passphrase = Some("open sesame".into());
    let view = keychain_create(&f.state, input).unwrap();
    assert!(view.has_passphrase);
    let row = record(&f.state, &view.id);
    let passphrase_ref = row.passphrase_ref.unwrap();
    assert_eq!(
        secret(&f.state, &passphrase_ref).as_deref(),
        Some("open sesame")
    );
    keychain_delete(&f.state, &view.id).unwrap();
    assert_eq!(secret(&f.state, &passphrase_ref), None);
}

#[test]
fn bad_input_is_refused_and_leaves_nothing_behind() {
    let f = fixture();
    let key = key();
    // No label: the row is refused after the secrets were written.
    let error = keychain_create(&f.state, new_key(" ", &key)).err().unwrap();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    // A public key that belongs to another private key.
    let mut input = new_key("mismatch", &key);
    input.public_key = Some(self::key().public_key().to_openssh().unwrap());
    assert!(keychain_create(&f.state, input).is_err());
    // Not a key at all, and the text is not echoed.
    let mut input = new_key("junk", &key);
    input.private_key = "certainly not a key".into();
    let error = keychain_create(&f.state, input).err().unwrap();
    assert!(!error.message.contains("certainly"), "{error}");
    assert!(keychain_list(&f.state).unwrap().is_empty());
}

#[test]
fn a_key_a_host_signs_in_with_cannot_be_deleted() {
    let mut f = fixture();
    let view = keychain_create(&f.state, new_key("deploy", &key())).unwrap();
    let host = HostUpsert {
        display_name: "Build box".into(),
        os_family: "linux".into(),
        protocol: Protocol::Ssh,
        address: "192.0.2.10".into(),
        port: 22,
        username: "dev".into(),
        auth: SshAuth::Keychain {
            key_id: view.id.clone(),
        },
        group_name: None,
        tags: Vec::new(),
        startup_snippet: None,
    };
    let host = f.state.db.get_mut().unwrap().create_host(&host).unwrap();
    assert_eq!(keychain_list(&f.state).unwrap()[0].used_by, ["Build box"]);

    let error = keychain_delete(&f.state, &view.id).err().unwrap();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("Build box"), "{error}");
    let private_ref = record(&f.state, &view.id).private_ref.unwrap();
    assert!(secret(&f.state, &private_ref).is_some());

    f.state.db.get_mut().unwrap().delete_host(&host.id).unwrap();
    keychain_delete(&f.state, &view.id).unwrap();
}

#[test]
fn replacing_the_private_key_swaps_the_secret_and_removes_the_old_one() {
    let f = fixture();
    let view = keychain_create(&f.state, new_key("rotating", &key())).unwrap();
    let old_ref = record(&f.state, &view.id).private_ref.unwrap();

    let next = key();
    let update = KeyUpdate {
        label: "rotated".into(),
        private_key: Some(text(&next)),
        ..Default::default()
    };
    let updated = keychain_update(&f.state, &view.id, update).unwrap();
    assert_eq!(updated.label, "rotated");
    assert!(same_public_key(
        &updated.public_key,
        &next.public_key().to_openssh().unwrap()
    ));
    let new_ref = record(&f.state, &view.id).private_ref.unwrap();
    assert_ne!(new_ref, old_ref);
    assert_eq!(secret(&f.state, &old_ref), None);
    assert_eq!(secret(&f.state, &new_ref), Some(text(&next)));

    // A bad replacement changes nothing.
    let update = KeyUpdate {
        label: "rotated".into(),
        private_key: Some("not a key".into()),
        ..Default::default()
    };
    assert!(keychain_update(&f.state, &view.id, update).is_err());
    assert_eq!(record(&f.state, &view.id).private_ref.unwrap(), new_ref);
    keychain_delete(&f.state, &view.id).unwrap();
}

#[test]
fn a_fido2_key_is_added_by_its_public_line_only() {
    let f = fixture();
    let sk = PublicKey::from(KeyData::SkEd25519(SkEd25519::new(
        Ed25519PublicKey([9u8; 32]),
        "ssh:",
    )));
    let line = sk.to_openssh().unwrap();
    let input = |public_key: Option<String>| KeyCreate {
        label: "YubiKey".into(),
        kind: KeyKind::Fido2,
        public_key,
        ..Default::default()
    };
    assert!(keychain_create(&f.state, input(None)).is_err());
    // An ordinary public key is not a security key.
    let plain = key().public_key().to_openssh().unwrap();
    assert!(keychain_create(&f.state, input(Some(plain))).is_err());

    let view = keychain_create(&f.state, input(Some(line))).unwrap();
    assert_eq!(view.kind, KeyKind::Fido2);
    assert_eq!(view.algorithm, "sk-ssh-ed25519@openssh.com");
    assert!(!view.has_private_key && !view.has_passphrase);
    keychain_delete(&f.state, &view.id).unwrap();
}

#[test]
fn a_key_file_is_read_with_the_public_key_next_to_it() {
    let f = fixture();
    let key = key();
    let path = f.dir.join("id_test");
    std::fs::write(&path, text(&key)).unwrap();
    let public = key.public_key().to_openssh().unwrap();
    std::fs::write(f.dir.join("id_test.pub"), format!("{public}\n")).unwrap();

    let import = keychain_import_file(&path).unwrap();
    assert_eq!(import.label, "id_test");
    assert_eq!(import.private_key, text(&key));
    assert_eq!(import.public_key.as_deref(), Some(public.as_str()));
    assert_eq!(import.certificate, None);

    // The public half alone is not a key file; neither is a missing file,
    // and the path is not repeated in the message.
    assert!(keychain_import_file(&f.dir.join("id_test.pub")).is_err());
    let error = keychain_import_file(&f.dir.join("nowhere")).err().unwrap();
    assert!(!error.message.contains("nowhere"), "{error}");
}

#[test]
fn a_key_file_tells_whether_it_asks_for_a_passphrase_without_returning_the_key() {
    let dir = std::env::temp_dir().join(format!("svk-keyfile-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = key();
    let locked = key.encrypt(&mut rand::rng(), "open sesame").unwrap();
    std::fs::write(dir.join("plain"), text(&key)).unwrap();
    std::fs::write(dir.join("locked"), text(&locked)).unwrap();
    std::fs::write(dir.join("notes.txt"), "hello").unwrap();

    assert_eq!(key_file_is_encrypted(&dir.join("plain")), Ok(false));
    assert_eq!(key_file_is_encrypted(&dir.join("locked")), Ok(true));
    // Not a key file, or not there: no answer, and no path in the message.
    assert!(key_file_is_encrypted(&dir.join("notes.txt")).is_err());
    let error = key_file_is_encrypted(&dir.join("nowhere")).unwrap_err();
    assert!(!error.message.contains("nowhere"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}
