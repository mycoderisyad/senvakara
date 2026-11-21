//! Local terminal sessions (TERM-01, TERM-03 / task CORE-04).
//!
//! Output bytes go to the webview through a `Channel` as raw `ArrayBuffer`s
//! (not JSON events). Each session is torn down on close; dropping the
//! `PtySession` kills its child so no shells are orphaned (NFR-02).

use senvakara_adapters_pty::{PtySession, ShellKind};
use senvakara_core::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, PtySession>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShellRequest {
    PowerShell,
    Wsl { distro: Option<String> },
}

#[derive(Debug, Clone, Serialize)]
pub struct TermExit {
    pub id: u32,
}

fn internal(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal(e.to_string())
}

fn not_found(id: u32) -> CoreError {
    CoreError::new(ErrorCode::Conflict, format!("terminal {id} is not open"))
}

#[tauri::command]
pub fn term_open(
    app: AppHandle,
    state: State<'_, Terminals>,
    shell: ShellRequest,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
) -> Result<u32, CoreError> {
    let kind = match shell {
        ShellRequest::PowerShell => ShellKind::PowerShell,
        ShellRequest::Wsl { distro } => ShellKind::Wsl { distro },
    };
    let session = PtySession::spawn(&kind, &[], cols.max(2), rows.max(2)).map_err(internal)?;
    let mut reader = session.reader().map_err(internal)?;
    let id = state.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    state.sessions.lock().unwrap().insert(id, session);

    std::thread::Builder::new()
        .name(format!("pty-reader-{id}"))
        .spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if on_data
                            .send(InvokeResponseBody::Raw(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            let _ = app.emit("term-exit", TermExit { id });
        })
        .map_err(internal)?;
    Ok(id)
}

#[tauri::command]
pub fn term_write(state: State<'_, Terminals>, id: u32, data: String) -> Result<(), CoreError> {
    let mut sessions = state.sessions.lock().unwrap();
    let s = sessions.get_mut(&id).ok_or_else(|| not_found(id))?;
    s.write_input(data.as_bytes()).map_err(internal)
}

#[tauri::command]
pub fn term_resize(
    state: State<'_, Terminals>,
    id: u32,
    cols: u16,
    rows: u16,
) -> Result<(), CoreError> {
    let sessions = state.sessions.lock().unwrap();
    let s = sessions.get(&id).ok_or_else(|| not_found(id))?;
    s.resize(cols.max(2), rows.max(2)).map_err(internal)
}

#[tauri::command]
pub fn term_close(state: State<'_, Terminals>, id: u32) -> Result<(), CoreError> {
    // Dropping the session kills the child process and closes the PTY.
    let removed = state.sessions.lock().unwrap().remove(&id);
    if let Some(mut s) = removed {
        s.kill().map_err(internal)?;
    }
    Ok(())
}

/// Installed WSL distributions (`wsl.exe -l -q` prints UTF-16LE).
#[tauri::command]
pub async fn wsl_distros() -> Result<Vec<String>, CoreError> {
    let out = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .output()
        .map_err(|e| {
            CoreError::new(
                ErrorCode::UnsupportedCapability,
                format!("WSL unavailable: {e}"),
            )
        })?;
    Ok(parse_wsl_list(&out.stdout))
}

fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes[1] == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.lines()
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_wsl_list;

    #[test]
    fn parses_utf16_wsl_output() {
        let s = "Ubuntu-24.04\r\ndocker-desktop\r\n";
        let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-24.04", "docker-desktop"]);
    }
}
