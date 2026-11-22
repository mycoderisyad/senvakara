//! Passwords of a host (CRED-01, DESK-IMPL-04). An SSH password is filed in
//! the OS credential store under an opaque reference. A Remote Desktop
//! password is written where `mstsc.exe` looks for it, `TERMSRV/<address>`
//! (ADR-026). Neither is ever read back for the UI.

use super::host_view;
use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
use crate::{App, HostView};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_secrets::SecretStore;
use senvakara_storage::RdpEndpoint;
use std::sync::Arc;

fn store(state: &AppState) -> CmdResult<&SecretStore> {
    state.secrets.as_ref().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "credential store unavailable",
        )
    })
}

/// Save (or clear) the SSH login password in Windows Credential Manager.
pub(crate) async fn host_set_password(
    app: &Arc<App>,
    id: String,
    password: Option<String>,
) -> CmdResult<HostView> {
    let state = &app.state;
    let store = store(state)?;
    let old = state
        .db
        .lock()
        .unwrap()
        .get_host(&id)
        .map_err(|e| db_err("password", e))?
        .ssh
        .ok_or_else(|| CoreError::new(ErrorCode::InvalidInput, "host has no SSH endpoint"))?
        .password_ref;
    let new_ref = match password.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => Some(
            store
                .put(p)
                .map_err(|e| to_core("save password", e.into()))?,
        ),
        None => None,
    };
    let swapped = state
        .db
        .lock()
        .unwrap()
        .set_password_ref(&id, new_ref.as_deref());
    if let Err(error) = swapped {
        // The row does not point at the new secret: do not leave it behind.
        if let Some(reference) = &new_ref {
            let _ = store.delete(reference);
        }
        return Err(db_err("password", error));
    }
    if let Some(reference) = old {
        let _ = store.delete(&reference);
    }
    logging::info(&format!(
        "password {} for host {id}",
        if new_ref.is_some() {
            "saved"
        } else {
            "cleared"
        }
    ));
    state.disconnect(&id).await;
    host_view(state, &id)
}

/// Whether another Remote Desktop host still relies on `target`. Windows
/// keeps one credential per address, so hosts at the same address share it.
fn shared_target(state: &AppState, host_id: &str, target: &str) -> bool {
    let Ok(hosts) = state.db.lock().unwrap().list_hosts() else {
        // Unknown: keep the credential rather than break another host.
        return true;
    };
    hosts.iter().any(|host| {
        host.id != host_id
            && host
                .rdp
                .as_ref()
                .and_then(|rdp| rdp.credential_target.as_deref())
                == Some(target)
    })
}

/// Delete a `TERMSRV/...` entry unless another host still uses it.
pub(super) fn drop_rdp_credential(
    state: &AppState,
    store: &SecretStore,
    host_id: &str,
    target: &str,
) -> CmdResult<()> {
    if shared_target(state, host_id, target) {
        return Ok(());
    }
    store
        .delete_rdp_credential(target)
        .map_err(|e| to_core("delete remote desktop credential", e.into()))
}

/// Save (or clear) the password `mstsc.exe` signs in with. It is stored with
/// the host's user name under `TERMSRV/<address>`; an entry Windows already
/// had for that address is replaced.
pub(crate) fn host_set_rdp_password(
    state: &AppState,
    id: &str,
    password: Option<String>,
) -> CmdResult<HostView> {
    let store = store(state)?;
    let rdp = state
        .db
        .lock()
        .unwrap()
        .get_host(id)
        .map_err(|e| db_err("remote desktop password", e))?
        .rdp
        .ok_or_else(|| {
            CoreError::new(ErrorCode::InvalidInput, "host is not a remote desktop host")
        })?;
    let new_target = match password.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => {
            if rdp.username.is_empty() {
                return Err(CoreError::new(
                    ErrorCode::InvalidInput,
                    "add the user name before saving the password",
                ));
            }
            Some(
                store
                    .put_rdp_credential(&rdp.address, &rdp.username, p)
                    .map_err(|e| to_core("save remote desktop credential", e.into()))?,
            )
        }
        None => None,
    };
    state
        .db
        .lock()
        .unwrap()
        .set_rdp_credential_target(id, new_target.as_deref())
        .map_err(|e| db_err("remote desktop password", e))?;
    if let Some(old) = rdp
        .credential_target
        .filter(|old| Some(old) != new_target.as_ref())
    {
        drop_rdp_credential(state, store, id, &old)?;
    }
    logging::info(&format!(
        "remote desktop password {} for host {id}",
        if new_target.is_some() {
            "saved"
        } else {
            "cleared"
        }
    ));
    host_view(state, id)
}

/// After an edit: a saved Remote Desktop credential belongs to the old
/// address and user name. The password cannot be read back to move it, so
/// the old entry is removed and the host is marked as having none; the user
/// is asked for the password again.
pub(super) fn forget_moved_rdp_credential(
    state: &AppState,
    id: &str,
    before: Option<&RdpEndpoint>,
    after: Option<&RdpEndpoint>,
) -> CmdResult<()> {
    let (Some(before), Some(after)) = (before, after) else {
        return Ok(());
    };
    let Some(target) = before.credential_target.as_deref() else {
        return Ok(());
    };
    if before.address == after.address && before.username == after.username {
        return Ok(());
    }
    state
        .db
        .lock()
        .unwrap()
        .set_rdp_credential_target(id, None)
        .map_err(|e| db_err("remote desktop password", e))?;
    if let Some(store) = &state.secrets {
        drop_rdp_credential(state, store, id, target)?;
    }
    logging::info(&format!(
        "remote desktop password cleared for host {id}: its address or user changed"
    ));
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use senvakara_storage::{HostUpsert, Protocol, SshAuth};

    fn state() -> (std::path::PathBuf, AppState) {
        let dir = std::env::temp_dir().join(format!("svk-rdp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = AppState::open(dir.join("test.db")).unwrap();
        (dir, state)
    }

    fn rdp_host(address: &str, username: &str) -> HostUpsert {
        HostUpsert {
            display_name: "Office PC".into(),
            os_family: "windows".into(),
            protocol: Protocol::Rdp,
            address: address.into(),
            port: 3389,
            username: username.into(),
            auth: SshAuth::Password,
            group_name: None,
            tags: Vec::new(),
            startup_snippet: None,
        }
    }

    fn target(state: &AppState, id: &str) -> Option<String> {
        let host = state.db.lock().unwrap().get_host(id).unwrap();
        host.rdp.unwrap().credential_target
    }

    /// Against the real Windows Credential Manager, at an address that
    /// cannot exist; the entry is removed again.
    #[test]
    fn a_remote_desktop_password_is_filed_under_the_address_and_cleared_again() {
        let (dir, mut state) = state();
        let address = format!("senvakara-test-{}.invalid", uuid::Uuid::new_v4());
        let db = state.db.get_mut().unwrap();
        let host = db.create_host(&rdp_host(&address, "tester")).unwrap();
        let nameless = db.create_host(&rdp_host("192.0.2.9", "")).unwrap();

        let view = host_set_rdp_password(&state, &host.id, Some("not-a-real-password".into()));
        assert!(view.unwrap().rdp.unwrap().has_credential);
        assert_eq!(target(&state, &host.id), Some(format!("TERMSRV/{address}")));
        // Without a user name there is nothing to file the password under.
        let error = host_set_rdp_password(&state, &nameless.id, Some("x".into()));
        assert_eq!(error.err().unwrap().code, ErrorCode::InvalidInput);

        let view = host_set_rdp_password(&state, &host.id, None).unwrap();
        assert!(!view.rdp.unwrap().has_credential);
        assert_eq!(target(&state, &host.id), None);
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_credential_is_forgotten_when_the_address_or_user_changes() {
        let (dir, mut state) = state();
        let address = format!("senvakara-test-{}.invalid", uuid::Uuid::new_v4());
        let db = state.db.get_mut().unwrap();
        let host = db.create_host(&rdp_host(&address, "tester")).unwrap();
        let twin = db.create_host(&rdp_host(&address, "tester")).unwrap();
        host_set_rdp_password(&state, &host.id, Some("not-a-real-password".into())).unwrap();
        host_set_rdp_password(&state, &twin.id, Some("not-a-real-password".into())).unwrap();
        let saved = format!("TERMSRV/{address}");
        // Two hosts at one address share the entry Windows keeps for it.
        assert!(shared_target(&state, &host.id, &saved));

        let edit = |state: &mut AppState, id: &str, input: &HostUpsert| {
            let db = state.db.get_mut().unwrap();
            let before = db.get_host(id).unwrap();
            let after = db.update_host(id, input).unwrap();
            forget_moved_rdp_credential(state, id, before.rdp.as_ref(), after.rdp.as_ref())
                .unwrap();
        };
        // An edit that keeps address and user keeps the credential.
        let mut same = rdp_host(&address, "tester");
        same.display_name = "Renamed".into();
        edit(&mut state, &host.id, &same);
        assert_eq!(target(&state, &host.id), Some(saved.clone()));

        edit(&mut state, &host.id, &rdp_host(&address, "someone-else"));
        assert_eq!(target(&state, &host.id), None);
        // The twin still relies on the entry, so it was kept for it.
        assert_eq!(target(&state, &twin.id), Some(saved.clone()));
        assert!(!shared_target(&state, &twin.id, &saved));

        edit(&mut state, &twin.id, &rdp_host("192.0.2.77", "tester"));
        assert_eq!(target(&state, &twin.id), None);
        drop(state);
        let _ = std::fs::remove_dir_all(dir);
    }
}
