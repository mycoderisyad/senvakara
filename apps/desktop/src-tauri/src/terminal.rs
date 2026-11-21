//! Terminal sessions: local PTY (TERM-01) and remote SSH PTY (TERM-02),
//! one tab model for both (TERM-03, tasks CORE-04, CORE-05).
//!
//! Output bytes go to the webview through a `Channel` as raw `ArrayBuffer`s.
//! Closing a session kills the local process or closes the SSH channel
//! (NFR-02); the SSH connection itself stays pooled for other tabs.

use crate::logging;
use crate::state::{to_core, AppState, CmdResult};
use senvakara_adapters_pty::{PtySession, ShellKind};
use senvakara_adapters_ssh::{SshPtyEvent, SshPtyWriter};
use senvakara_core::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, State};

enum Session {
    Local(PtySession),
    Ssh(Arc<SshPtyWriter>),
}

#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Session>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShellRequest {
    PowerShell,
    Wsl {
        distro: Option<String>,
    },
    /// `tmux`: attach to (or create) that tmux session so the shell survives
    /// disconnects (REC-02); `None` opens a plain login shell.
    Ssh {
        host_id: String,
        tmux: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct TermExit {
    pub id: u32,
    pub exit_status: Option<u32>,
}

fn internal(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal(e.to_string())
}

fn not_open(id: u32) -> CoreError {
    CoreError::new(ErrorCode::Conflict, format!("terminal {id} is not open"))
}

#[tauri::command]
pub async fn term_open(
    app: AppHandle,
    terms: State<'_, Terminals>,
    state: State<'_, AppState>,
    shell: ShellRequest,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
) -> CmdResult<u32> {
    let (cols, rows) = (cols.max(2), rows.max(2));
    let id = terms.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    match shell {
        ShellRequest::Ssh { host_id, tmux } => {
            if let Some(name) = &tmux {
                if !valid_tmux_name(name) {
                    return Err(CoreError::new(
                        ErrorCode::InvalidInput,
                        "invalid tmux session name",
                    ));
                }
            }
            let conn = state.connection(&host_id).await?;
            let (writer, mut reader) = conn
                .open_pty_or_tmux(cols.into(), rows.into(), tmux.as_deref())
                .await
                .map_err(|e| to_core("open ssh shell", e))?;
            terms
                .sessions
                .lock()
                .unwrap()
                .insert(id, Session::Ssh(Arc::new(writer)));
            logging::info(&format!("ssh terminal {id} opened on host {host_id}"));
            tauri::async_runtime::spawn(async move {
                let mut exit_status = None;
                while let Some(ev) = reader.next().await {
                    match ev {
                        SshPtyEvent::Data(bytes) => {
                            if on_data.send(InvokeResponseBody::Raw(bytes)).is_err() {
                                break;
                            }
                        }
                        SshPtyEvent::Exit(code) => {
                            exit_status = exit_status.or(code);
                            if code.is_none() {
                                break;
                            }
                        }
                    }
                }
                let _ = app.emit("term-exit", TermExit { id, exit_status });
            });
        }
        local => {
            let kind = match local {
                ShellRequest::PowerShell => ShellKind::PowerShell,
                ShellRequest::Wsl { distro } => ShellKind::Wsl { distro },
                ShellRequest::Ssh { .. } => unreachable!(),
            };
            let session = PtySession::spawn(&kind, &[], cols, rows).map_err(internal)?;
            let mut reader = session.reader().map_err(internal)?;
            terms
                .sessions
                .lock()
                .unwrap()
                .insert(id, Session::Local(session));
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
                    let _ = app.emit(
                        "term-exit",
                        TermExit {
                            id,
                            exit_status: None,
                        },
                    );
                })
                .map_err(internal)?;
        }
    }
    Ok(id)
}

fn ssh_writer(terms: &Terminals, id: u32) -> CmdResult<Option<Arc<SshPtyWriter>>> {
    match terms.sessions.lock().unwrap().get(&id) {
        None => Err(not_open(id)),
        Some(Session::Ssh(w)) => Ok(Some(w.clone())),
        Some(Session::Local(_)) => Ok(None),
    }
}

#[tauri::command]
pub async fn term_write(terms: State<'_, Terminals>, id: u32, data: String) -> CmdResult<()> {
    match ssh_writer(&terms, id)? {
        Some(w) => w
            .write(data.as_bytes())
            .await
            .map_err(|e| to_core("ssh write", e)),
        None => match terms.sessions.lock().unwrap().get_mut(&id) {
            Some(Session::Local(s)) => s.write_input(data.as_bytes()).map_err(internal),
            _ => Err(not_open(id)),
        },
    }
}

#[tauri::command]
pub async fn term_resize(
    terms: State<'_, Terminals>,
    id: u32,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    let (cols, rows) = (cols.max(2), rows.max(2));
    match ssh_writer(&terms, id)? {
        Some(w) => w
            .resize(cols.into(), rows.into())
            .await
            .map_err(|e| to_core("ssh resize", e)),
        None => match terms.sessions.lock().unwrap().get(&id) {
            Some(Session::Local(s)) => s.resize(cols, rows).map_err(internal),
            _ => Err(not_open(id)),
        },
    }
}

#[tauri::command]
pub async fn term_close(terms: State<'_, Terminals>, id: u32) -> CmdResult<()> {
    let removed = terms.sessions.lock().unwrap().remove(&id);
    match removed {
        Some(Session::Local(mut s)) => s.kill().map_err(internal)?,
        Some(Session::Ssh(w)) => {
            let _ = w.close().await;
        }
        None => {}
    }
    Ok(())
}

/// tmux session names Senvakara creates: letters, digits, `-`, `_`.
pub fn valid_tmux_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

trait PtyOrTmux {
    async fn open_pty_or_tmux(
        &self,
        cols: u32,
        rows: u32,
        tmux: Option<&str>,
    ) -> anyhow::Result<(
        senvakara_adapters_ssh::SshPtyWriter,
        senvakara_adapters_ssh::SshPtyReader,
    )>;
}

impl PtyOrTmux for senvakara_adapters_ssh::SshConnection {
    async fn open_pty_or_tmux(
        &self,
        cols: u32,
        rows: u32,
        tmux: Option<&str>,
    ) -> anyhow::Result<(
        senvakara_adapters_ssh::SshPtyWriter,
        senvakara_adapters_ssh::SshPtyReader,
    )> {
        match tmux {
            Some(name) => {
                self.open_pty_command(cols, rows, &["tmux", "new-session", "-A", "-s", name])
                    .await
            }
            None => self.open_pty(cols, rows).await,
        }
    }
}

/// Installed WSL distributions (`wsl.exe -l -q` prints UTF-16LE).
#[tauri::command]
pub async fn wsl_distros() -> CmdResult<Vec<String>> {
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
    fn tmux_names() {
        assert!(super::valid_tmux_name("svk-fedora_1"));
        for bad in ["", "a b", "x;rm", "$(id)", &"a".repeat(41)] {
            assert!(!super::valid_tmux_name(bad), "{bad}");
        }
    }

    #[test]
    fn parses_utf16_wsl_output() {
        let s = "Ubuntu-24.04\r\ndocker-desktop\r\n";
        let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&bytes), ["Ubuntu-24.04", "docker-desktop"]);
    }
}
