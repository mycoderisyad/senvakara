//! Terminal sessions: local PTY (TERM-01) and remote SSH PTY (TERM-02),
//! one tab model for both (TERM-03, tasks CORE-04, CORE-05).
//!
//! Output goes to the session's [`Sink`] and ends with [`TermEvent::Exit`].
//! Input and resize are queued per session and applied in order by that
//! session's writer (a thread for a local PTY, a task for SSH), so a shell
//! that stops reading only stalls its own tab. Closing a session kills the
//! local process or closes the SSH channel (NFR-02); the SSH connection
//! itself stays pooled for other tabs. A session whose viewer is gone is
//! closed the same way.

use crate::events::TermEvent;
use crate::logging;
use crate::shells::LocalShell;
use crate::sink::Sink;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_pty::{PtySession, ShellKind};
use senvakara_adapters_ssh::{SshPtyEvent, SshPtyWriter};
use senvakara_core::{CoreError, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::AbortHandle;

/// Ordered work for a session's writer.
enum TermCmd {
    Input(Vec<u8>),
    Resize(u16, u16),
}

enum Session {
    Local {
        /// The only long-lived owner: dropping it closes the PTY, which also
        /// unblocks a writer stuck on a full pipe.
        pty: Arc<Mutex<PtySession>>,
        tx: std::sync::mpsc::Sender<TermCmd>,
    },
    Ssh {
        writer: Arc<SshPtyWriter>,
        tx: tokio::sync::mpsc::UnboundedSender<TermCmd>,
        write_task: AbortHandle,
    },
}

#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Session>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

fn internal(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal(e.to_string())
}

fn not_open(id: u32) -> CoreError {
    CoreError::new(ErrorCode::Conflict, format!("terminal {id} is not open"))
}

pub(crate) async fn term_open(
    app: &Arc<App>,
    shell: ShellRequest,
    cols: u16,
    rows: u16,
    sink: Sink<TermEvent>,
) -> CmdResult<u32> {
    let (cols, rows) = (cols.max(2), rows.max(2));
    let terms = &app.terms;
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
            let conn = app.state.connection(&host_id).await?;
            // Decided before the shell opens: for tmux this must see whether
            // the session already existed.
            let startup = startup_input(app, &conn, &host_id, tmux.as_deref()).await;
            let (writer, mut reader) = conn
                .open_pty_or_tmux(cols.into(), rows.into(), tmux.as_deref())
                .await
                .map_err(|e| to_core("open ssh shell", e))?;
            let writer = Arc::new(writer);
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<TermCmd>();
            // Queued before the session can be written to, so the snippet is
            // typed ahead of anything the user types.
            if let Some(bytes) = startup {
                let _ = tx.send(TermCmd::Input(bytes));
            }
            let w = writer.clone();
            let write_task = app.rt.spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let done = match cmd {
                        TermCmd::Input(bytes) => w.write(&bytes).await,
                        TermCmd::Resize(cols, rows) => w.resize(cols.into(), rows.into()).await,
                    };
                    if let Err(e) = done {
                        logging::warn(&format!("ssh terminal {id}: write failed: {e:#}"));
                        break;
                    }
                }
            });
            terms.sessions.lock().unwrap().insert(
                id,
                Session::Ssh {
                    writer,
                    tx,
                    write_task: write_task.abort_handle(),
                },
            );
            logging::info(&format!("ssh terminal {id} opened on host {host_id}"));
            let task_app = app.clone();
            app.rt.spawn(async move {
                let mut exit_status = None;
                while let Some(ev) = reader.next().await {
                    match ev {
                        SshPtyEvent::Data(bytes) => {
                            if !sink(TermEvent::Data(bytes)) {
                                // Nobody is looking any more: do not leave the shell running.
                                let _ = term_close(&task_app, id);
                                return;
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
                sink(TermEvent::Exit(exit_status));
            });
        }
        local => {
            let kind = match local {
                ShellRequest::PowerShell => ShellKind::PowerShell,
                ShellRequest::Wsl { distro } => ShellKind::Wsl { distro },
                ShellRequest::Ssh { .. } => unreachable!(),
            };
            let mut session = PtySession::spawn(&kind, &[], cols, rows).map_err(internal)?;
            let mut reader = session.reader().map_err(internal)?;
            let mut writer = session
                .take_writer()
                .ok_or_else(|| internal("pty input stream unavailable"))?;
            let pty = Arc::new(Mutex::new(session));
            let (tx, rx) = std::sync::mpsc::channel::<TermCmd>();
            // The writer must not keep the PTY alive: see `Session::Local::pty`.
            let weak = Arc::downgrade(&pty);
            std::thread::Builder::new()
                .name(format!("pty-writer-{id}"))
                .spawn(move || {
                    for cmd in rx {
                        match cmd {
                            TermCmd::Input(bytes) => {
                                let done = writer.write_all(&bytes).and_then(|_| writer.flush());
                                if done.is_err() {
                                    break;
                                }
                            }
                            TermCmd::Resize(cols, rows) => {
                                let Some(pty) = weak.upgrade() else { break };
                                let _ = pty.lock().unwrap().resize(cols, rows);
                            }
                        }
                    }
                })
                .map_err(internal)?;
            terms
                .sessions
                .lock()
                .unwrap()
                .insert(id, Session::Local { pty, tx });
            let task_app = app.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("pty-reader-{id}"))
                .spawn(move || {
                    let mut buf = vec![0u8; 16 * 1024];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if !sink(TermEvent::Data(buf[..n].to_vec())) {
                                    let _ = term_close(&task_app, id);
                                    return;
                                }
                            }
                        }
                    }
                    sink(TermEvent::Exit(None));
                });
            if let Err(e) = spawned {
                let _ = term_close(app, id);
                return Err(internal(e));
            }
        }
    }
    Ok(id)
}

fn queue(terms: &Terminals, id: u32, cmd: TermCmd) -> CmdResult<()> {
    let sent = match terms.sessions.lock().unwrap().get(&id) {
        None => false,
        Some(Session::Local { tx, .. }) => tx.send(cmd).is_ok(),
        Some(Session::Ssh { tx, .. }) => tx.send(cmd).is_ok(),
    };
    if sent {
        Ok(())
    } else {
        Err(not_open(id))
    }
}

/// Queue input for the session's writer; returns at once.
pub(crate) fn term_write(app: &App, id: u32, data: Vec<u8>) -> CmdResult<()> {
    queue(&app.terms, id, TermCmd::Input(data))
}

/// Queue a resize behind the input already queued; returns at once.
pub(crate) fn term_resize(app: &App, id: u32, cols: u16, rows: u16) -> CmdResult<()> {
    queue(&app.terms, id, TermCmd::Resize(cols.max(2), rows.max(2)))
}

pub(crate) fn term_close(app: &App, id: u32) -> CmdResult<()> {
    let removed = app.terms.sessions.lock().unwrap().remove(&id);
    match removed {
        // Dropping `pty` afterwards closes the PTY and ends both threads.
        Some(Session::Local { pty, .. }) => pty.lock().unwrap().kill().map_err(internal)?,
        Some(Session::Ssh {
            writer, write_task, ..
        }) => {
            write_task.abort();
            app.rt.spawn(async move {
                let _ = writer.close().await;
            });
        }
        None => {}
    }
    Ok(())
}

/// What to type into a new SSH shell: the host's startup snippet, once
/// (HOST-UI-02). A tmux tab re-attaches to its session after a reconnect, and
/// typing the snippet into a shell that already ran it would run it twice, so
/// it is only sent when the tmux session does not exist yet. The snippet is
/// the user's own text and is never logged.
async fn startup_input(
    app: &App,
    conn: &senvakara_adapters_ssh::SshConnection,
    host_id: &str,
    tmux: Option<&str>,
) -> Option<Vec<u8>> {
    let snippet = app
        .state
        .db
        .lock()
        .unwrap()
        .get_host(host_id)
        .ok()?
        .ssh?
        .startup_snippet?;
    if let Some(name) = tmux {
        // `=name` is an exact match; a plain name would match by prefix.
        let exact = format!("={name}");
        let found = conn.exec(&["tmux", "has-session", "-t", &exact]).await;
        // If it cannot be told, assume the session exists and type nothing.
        if found.map(|out| out.success()).unwrap_or(true) {
            return None;
        }
    }
    snippet_keys(&snippet)
}

/// The snippet as keystrokes: each line ends with Enter, as if typed.
fn snippet_keys(snippet: &str) -> Option<Vec<u8>> {
    let text = snippet.trim_end_matches(['\r', '\n']);
    if text.trim().is_empty() {
        return None;
    }
    let mut keys = text.replace("\r\n", "\r").replace('\n', "\r");
    keys.push('\r');
    Some(keys.into_bytes())
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

/// Installed WSL distributions, default first.
pub(crate) async fn wsl_distros() -> CmdResult<Vec<String>> {
    // `wsl.exe` can take a moment to answer: keep it off the async workers.
    tokio::task::spawn_blocking(crate::shells::wsl_list)
        .await
        .map_err(internal)?
        .map_err(|e| {
            CoreError::new(
                ErrorCode::UnsupportedCapability,
                format!("WSL unavailable: {e}"),
            )
        })
}

/// The shells installed on this PC (see [`crate::shells`]).
pub(crate) async fn local_shells() -> CmdResult<Vec<LocalShell>> {
    // Reads the file system and waits for `wsl.exe`.
    tokio::task::spawn_blocking(crate::shells::detect)
        .await
        .map_err(internal)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tmux_names() {
        assert!(super::valid_tmux_name("svk-fedora_1"));
        for bad in ["", "a b", "x;rm", "$(id)", &"a".repeat(41)] {
            assert!(!super::valid_tmux_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_startup_snippet_is_typed_line_by_line_with_enter() {
        let keys = |s: &str| super::snippet_keys(s).map(|b| String::from_utf8(b).unwrap());
        assert_eq!(keys("cd /srv").as_deref(), Some("cd /srv\r"));
        assert_eq!(
            keys("cd /srv\r\nsource env.sh\n\n").as_deref(),
            Some("cd /srv\rsource env.sh\r")
        );
        assert_eq!(keys(" \n"), None);
        assert_eq!(keys(""), None);
    }
}
