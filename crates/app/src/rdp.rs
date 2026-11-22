//! Remote Desktop hosts open in Microsoft's own client (DESK-IMPL-04,
//! ADR-026): Senvakara starts `mstsc.exe` for the host's address and lets it
//! find the saved credential under `TERMSRV/<address>` by itself. No password
//! passes through here.

use crate::logging;
use crate::state::{db_err, CmdResult};
use crate::App;
use senvakara_core::{CoreError, ErrorCode};
use std::path::PathBuf;
use std::sync::Arc;

/// The arguments `mstsc.exe` gets for one host: `/v:<address>:<port>`, with
/// an IPv6 address in brackets. One argument, so nothing in the address can
/// become another switch.
fn mstsc_args(address: &str, port: u16) -> Vec<String> {
    let host = if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    };
    vec![format!("/v:{host}:{port}")]
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
    let args = mstsc_args(&rdp.address, rdp.port);
    // Starting a process can take a moment: keep it off the async workers.
    let started = tokio::task::spawn_blocking(move || {
        // Executable plus argument list, never a shell string. The child is
        // not waited for: closing Senvakara leaves the session open.
        std::process::Command::new(mstsc_path())
            .args(args)
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
    use super::mstsc_args;

    #[test]
    fn the_address_and_port_form_one_argument() {
        assert_eq!(mstsc_args("192.0.2.7", 3389), ["/v:192.0.2.7:3389"]);
        assert_eq!(mstsc_args("pc.example", 3390), ["/v:pc.example:3390"]);
    }

    #[test]
    fn an_ipv6_address_is_bracketed() {
        assert_eq!(mstsc_args("2001:db8::7", 3389), ["/v:[2001:db8::7]:3389"]);
        assert_eq!(mstsc_args("[2001:db8::7]", 3389), ["/v:[2001:db8::7]:3389"]);
    }
}
