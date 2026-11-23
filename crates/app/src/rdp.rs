//! Remote Desktop hosts open in Microsoft's own client (DESK-IMPL-04,
//! ADR-026): Senvakara writes a small `.rdp` file with the host's address and
//! user name, starts `mstsc.exe` on it, and lets the client find the saved
//! credential under `TERMSRV/<address>` by itself. No password passes through
//! here or into the file.

use crate::logging;
use crate::state::{db_err, CmdResult};
use crate::App;
use senvakara_core::{CoreError, ErrorCode};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The text of the `.rdp` file for one host: the address with its port (an
/// IPv6 address in brackets) and, when there is one, the user name. Nothing
/// that changes how the connection is secured is written. A control character
/// in a value would start another line of the file, so it is refused.
fn rdp_file_text(address: &str, port: u16, username: &str) -> CmdResult<String> {
    for (what, value) in [("address", address), ("user name", username)] {
        if value.chars().any(char::is_control) {
            return Err(CoreError::new(
                ErrorCode::InvalidInput,
                format!("the {what} contains a control character"),
            ));
        }
    }
    if address.is_empty() {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "the address is empty",
        ));
    }
    let host = if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    };
    let mut text = format!("full address:s:{host}:{port}\r\n");
    if !username.is_empty() {
        text.push_str(&format!("username:s:{username}\r\n"));
    }
    Ok(text)
}

/// `<data folder>/rdp/<hash of the host id>.rdp`: one file per host, written
/// again on every launch. The name comes from a hash, so no user text is in it.
fn rdp_file_path(data_dir: &Path, host_id: &str) -> PathBuf {
    let hash: String = Sha256::digest(host_id.as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    data_dir.join("rdp").join(format!("{hash}.rdp"))
}

/// `mstsc.exe` from the Windows system folder, so a program of the same name
/// elsewhere on `PATH` is never the one started.
fn mstsc_path() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("mstsc.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("mstsc.exe"))
}

/// Open the host in Remote Desktop Connection. Returns as soon as the client
/// has started; its window lives on its own.
pub(crate) async fn rdp_launch(app: &Arc<App>, host_id: String) -> CmdResult<()> {
    let host = app
        .state
        .db
        .lock()
        .unwrap()
        .get_host(&host_id)
        .map_err(|e| db_err("remote desktop", e))?;
    let rdp = host.rdp.ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "this host is not a remote desktop host",
        )
    })?;
    if !cfg!(windows) {
        return Err(CoreError::new(
            ErrorCode::UnsupportedCapability,
            "Remote Desktop Connection is only available on Windows",
        ));
    }
    let text = rdp_file_text(&rdp.address, rdp.port, &rdp.username)?;
    let file = rdp_file_path(&app.paths.data_dir, &host_id);
    // Writing the file and starting a process can take a moment: keep them
    // off the async workers.
    let started = tokio::task::spawn_blocking(move || {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&file, text)?;
        // Executable plus argument list, never a shell string. The child is
        // not waited for: closing Senvakara leaves the session open.
        std::process::Command::new(mstsc_path())
            .arg(&file)
            .spawn()
            .map(drop)
    })
    .await;
    match started {
        Ok(Ok(())) => {
            let _ = app.state.db.lock().unwrap().mark_seen(&host_id);
            logging::info(&format!("remote desktop opened for host {host_id}"));
            Ok(())
        }
        Ok(Err(error)) => {
            logging::warn(&format!(
                "remote desktop for host {host_id}: cannot start mstsc.exe: {error}"
            ));
            Err(CoreError::new(
                ErrorCode::UnsupportedCapability,
                "Remote Desktop Connection (mstsc.exe) could not be started",
            ))
        }
        Err(error) => Err(CoreError::internal(format!(
            "background task failed: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_carries_the_address_port_and_user() {
        assert_eq!(
            rdp_file_text("192.0.2.7", 3389, "alice").unwrap(),
            "full address:s:192.0.2.7:3389\r\nusername:s:alice\r\n"
        );
        assert_eq!(
            rdp_file_text("pc.example", 3390, "CORP\\bob").unwrap(),
            "full address:s:pc.example:3390\r\nusername:s:CORP\\bob\r\n"
        );
    }

    #[test]
    fn an_empty_user_name_leaves_the_user_line_out() {
        assert_eq!(
            rdp_file_text("192.0.2.7", 3389, "").unwrap(),
            "full address:s:192.0.2.7:3389\r\n"
        );
    }

    #[test]
    fn an_ipv6_address_is_bracketed() {
        let text = rdp_file_text("2001:db8::7", 3389, "").unwrap();
        assert_eq!(text, "full address:s:[2001:db8::7]:3389\r\n");
        assert_eq!(rdp_file_text("[2001:db8::7]", 3389, "").unwrap(), text);
    }

    #[test]
    fn a_line_break_in_a_value_is_refused() {
        for (address, user) in [
            ("pc\nusername:s:admin", "alice"),
            ("pc\r\ngateway hostname:s:evil", "alice"),
            ("pc", "alice\nenablecredsspsupport:i:0"),
            ("pc", "alice\r"),
            ("pc", "al\u{0}ice"),
        ] {
            let error = rdp_file_text(address, 3389, user).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidInput);
        }
        assert_eq!(
            rdp_file_text("", 3389, "alice").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn the_file_name_is_a_hash_of_the_host_id_only() {
        let dir = Path::new("data");
        let first = rdp_file_path(dir, "host-1");
        assert_eq!(first, rdp_file_path(dir, "host-1"));
        assert_ne!(first, rdp_file_path(dir, "host-2"));
        let name = first.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 32 + ".rdp".len());
        assert!(name.ends_with(".rdp") && name[..32].bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(first.parent().unwrap(), dir.join("rdp"));
        // Even a host id with path characters cannot leave the folder.
        let odd = rdp_file_path(dir, "..\\..\\x/y");
        assert_eq!(odd.parent().unwrap(), dir.join("rdp"));
    }
}
