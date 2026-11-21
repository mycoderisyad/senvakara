//! Remote desktop of a Linux host's *current* graphical session
//! (DESK-01..03, tasks DESK-IMPL-02/03, POC-07).
//!
//! The target decides what is available: a host in text mode offers only
//! the terminal; a host with an active X11 session also offers its screen.
//!
//! Security (docs/SECURITY.md §3, §4): x11vnc is started on demand over the
//! authenticated SSH connection, listens on the host's loopback only (a
//! random port, verified with `ss` right after start; any other listener
//! stops the session), requires a one-time 8-character password delivered
//! over SFTP into the user's 0700 `$XDG_RUNTIME_DIR` and deleted by x11vnc
//! on read, serves one client and is stopped when the session closes. The
//! viewer reaches it with `conn.open_tcp("127.0.0.1", vnc_port)` (SSH
//! `direct-tcpip`); how the pixels get to the screen is the front end's
//! business. (x11vnc's `-unixsock` needs a patched libvncserver and
//! `-rfbport 0` still listened on `[::]:5900` on Fedora 45, so the
//! Unix-socket design was dropped.)

use crate::logging;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_ssh::{SshConnection, SshPtyWriter};
use senvakara_core::{CoreError, ErrorCode};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::task::AbortHandle;

/// Fixed detection script: no user input is interpolated.
const DETECT: &str = r#"me=$(id -un); for s in $(loginctl list-sessions --no-legend 2>/dev/null | awk '{print $1}'); do loginctl show-session "$s" -p Name -p Type -p Display -p Active -p Desktop 2>/dev/null; echo '---'; done; echo "@@ME=$me"; echo "@@RUNTIME=$XDG_RUNTIME_DIR"; echo "@@X11VNC=$(command -v x11vnc)"; for p in $(pgrep -u "$(id -u)"); do x=$( { tr '\0' '\n' < /proc/$p/environ; } 2>/dev/null | grep -m1 '^XAUTHORITY='); if [ -n "$x" ]; then echo "@@XAUTH=${x#XAUTHORITY=}"; break; fi; done"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct DesktopInfo {
    /// `x11`, `wayland` or `none`.
    pub session: String,
    pub display: Option<String>,
    pub desktop: Option<String>,
    pub x11vnc: Option<String>,
    #[serde(skip)]
    pub runtime_dir: Option<String>,
    /// X authority file of the user's session. Display managers put it in
    /// different places (lightdm: /run/lightdm/<user>/xauthority), so it is
    /// read from the user's own processes instead of guessed.
    #[serde(skip)]
    pub xauthority: Option<String>,
}

pub fn parse_detect(text: &str) -> DesktopInfo {
    let mut info = DesktopInfo {
        session: "none".into(),
        ..Default::default()
    };
    let me = text
        .lines()
        .find_map(|l| l.strip_prefix("@@ME="))
        .unwrap_or("")
        .trim()
        .to_string();
    info.runtime_dir = text
        .lines()
        .find_map(|l| l.strip_prefix("@@RUNTIME="))
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .map(str::to_string);
    info.x11vnc = text
        .lines()
        .find_map(|l| l.strip_prefix("@@X11VNC="))
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .map(str::to_string);
    info.xauthority = text
        .lines()
        .find_map(|l| l.strip_prefix("@@XAUTH="))
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .map(str::to_string);
    for block in text.split("---") {
        let get = |k: &str| {
            block
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .map(str::trim)
                .unwrap_or("")
        };
        if get("Name") != me || get("Active") != "yes" {
            continue;
        }
        match get("Type") {
            "x11" if get("Display").starts_with(':') => {
                info.session = "x11".into();
                info.display = Some(get("Display").to_string());
                info.desktop = Some(get("Desktop").to_string()).filter(|d| !d.is_empty());
                break;
            }
            "wayland" if info.session == "none" => {
                info.session = "wayland".into();
                info.desktop = Some(get("Desktop").to_string()).filter(|d| !d.is_empty());
            }
            _ => {}
        }
    }
    info
}

pub async fn detect(conn: &SshConnection) -> anyhow::Result<DesktopInfo> {
    let out = conn.exec(&["sh", "-c", DETECT]).await?;
    Ok(parse_detect(&out.stdout_str()))
}

struct Session {
    conn: Arc<SshConnection>,
    vnc_port: u16,
    /// The x11vnc process channel; closing it ends x11vnc.
    child: SshPtyWriter,
    drain: AbortHandle,
}

#[derive(Default)]
pub struct Desktops {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Session>>,
}

/// 256-bit one-time token from the OS RNG (uuid v4 uses `getrandom`).
pub fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub(crate) async fn desktop_info(app: &Arc<App>, host_id: String) -> CmdResult<DesktopInfo> {
    let conn = app.state.connection(&host_id).await?;
    detect(&conn)
        .await
        .map_err(|e| to_core("detect desktop", e))
}

/// Loopback-only check of `ss -ltnH` output for one port: `Some(true)` when
/// every listener is on 127.0.0.1/::1, `Some(false)` if anything else
/// listens, `None` while nothing listens yet.
pub fn listeners_loopback_only(ss_output: &str, port: u16) -> Option<bool> {
    let suffix = format!(":{port}");
    let locals: Vec<&str> = ss_output
        .lines()
        .filter_map(|l| l.split_whitespace().nth(3))
        .filter(|a| a.ends_with(&suffix))
        .collect();
    if locals.is_empty() {
        return None;
    }
    Some(
        locals
            .iter()
            .all(|a| a.starts_with("127.") || a.starts_with("[::1]")),
    )
}

/// 20000..=59999 derived from random hex.
fn random_port(hex: &str) -> u16 {
    let n = u32::from_str_radix(&hex[..8], 16).unwrap_or(0);
    20000 + (n % 40000) as u16
}

/// Stop the x11vnc started for `port` (matched by its unique port argument).
pub async fn stop_x11vnc(conn: &SshConnection, port: u16) {
    let script = format!("pkill -u \"$(id -u)\" -f 'x11vnc .*-rfbport {port} '");
    let _ = conn.exec(&["sh", "-c", &script]).await;
}

/// A running x11vnc that serves the host's X11 session on the host's loopback only,
/// as started by [`start_vnc`]. Connect to it with `conn.open_tcp("127.0.0.1", vnc_port)`,
/// keep reading `child_reader`, and call [`stop_x11vnc`] when done.
pub struct VncServer {
    pub vnc_port: u16,
    /// One-time VNC password for this session only (x11vnc exits after it).
    pub password: String,
    pub display: String,
    pub desktop: Option<String>,
    pub child_writer: senvakara_adapters_ssh::SshPtyWriter,
    pub child_reader: senvakara_adapters_ssh::SshPtyReader,
}

/// Start x11vnc for the host's current X11 session and verify it listens on loopback only.
pub async fn start_vnc(conn: &Arc<SshConnection>) -> CmdResult<VncServer> {
    let info = detect(conn)
        .await
        .map_err(|e| to_core("detect desktop", e))?;
    let display = match info.session.as_str() {
        "x11" => info.display.clone().unwrap_or_default(),
        "wayland" => return Err(CoreError::new(
            ErrorCode::UnsupportedCapability,
            "the host runs a Wayland session; screen sharing currently supports X11 sessions only",
        )),
        _ => {
            return Err(CoreError::new(
                ErrorCode::UnsupportedCapability,
                "no graphical session is running on the host (text mode); use the terminal",
            ))
        }
    };
    let x11vnc = info.x11vnc.clone().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "x11vnc is not installed on the host (sudo dnf install x11vnc)",
        )
    })?;
    let runtime = info.runtime_dir.clone().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "the host has no private XDG_RUNTIME_DIR for the one-time password",
        )
    })?;

    // One-time secrets: file-name token (256 bit), VNC password (VNC auth uses
    // 8 characters), random loopback port on the host.
    let token = random_token();
    let password = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let vnc_port = random_port(&token[32..]);
    let pw_file = format!("{runtime}/senvakara-vnc-{}.pw", &token[..12]);

    // The password goes over SFTP into the 0700 runtime dir, never onto a
    // command line where other local users could see it via `ps`.
    {
        let sftp = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
        let mut f = sftp
            .create(pw_file.clone())
            .await
            .map_err(|e| to_core("password file", e.into()))?;
        f.write_all(password.as_bytes())
            .await
            .map_err(|e| to_core("password file", e.into()))?;
        f.shutdown()
            .await
            .map_err(|e| to_core("password file", e.into()))?;
    }

    let port_arg = vnc_port.to_string();
    let pw_arg = format!("rm:{pw_file}");
    // Loopback only, one client then exit, gone after 60 s without a client.
    let (child_writer, mut child_reader) = conn
        .spawn(&[
            &x11vnc,
            "-display",
            &display,
            "-auth",
            info.xauthority.as_deref().unwrap_or("guess"),
            "-localhost",
            "-rfbport",
            &port_arg,
            "-passwdfile",
            &pw_arg,
            "-once",
            "-timeout",
            "60",
            "-shared",
            "-noxdamage",
            "-nolookup",
            "-quiet",
        ])
        .await
        .map_err(|e| to_core("start x11vnc", e))?;

    // Wait until it listens, then insist that it listens on loopback only.
    let mut verdict = None;
    for _ in 0..40 {
        let ss = conn
            .exec(&["ss", "-ltnH"])
            .await
            .map_err(|e| to_core("x11vnc", e))?;
        verdict = listeners_loopback_only(&ss.stdout_str(), vnc_port);
        if verdict.is_some() {
            break;
        }
        if let Ok(Some(senvakara_adapters_ssh::SshPtyEvent::Exit(code))) =
            tokio::time::timeout(Duration::from_millis(150), child_reader.next()).await
        {
            let _ = conn.exec(&["rm", "-f", &pw_file]).await;
            return Err(CoreError::new(
                ErrorCode::InternalError,
                format!("x11vnc exited early (status {code:?})"),
            ));
        }
    }
    match verdict {
        Some(true) => {}
        Some(false) => {
            stop_x11vnc(conn, vnc_port).await;
            let _ = child_writer.close().await;
            logging::warn("desktop: x11vnc listened beyond loopback; stopped");
            return Err(CoreError::new(
                ErrorCode::Unauthorized,
                "x11vnc listened on a non-loopback address; it was stopped for safety",
            ));
        }
        None => {
            stop_x11vnc(conn, vnc_port).await;
            let _ = child_writer.close().await;
            let _ = conn.exec(&["rm", "-f", &pw_file]).await;
            return Err(CoreError::new(
                ErrorCode::Timeout,
                "x11vnc did not start within a few seconds",
            ));
        }
    }

    Ok(VncServer {
        vnc_port,
        password,
        display,
        desktop: info.desktop,
        child_writer,
        child_reader,
    })
}

/// An open remote-desktop session: x11vnc is running and waits for its one
/// client on the host's loopback. Reach it with
/// `conn.open_tcp("127.0.0.1", vnc_port)` and authenticate with `password`.
/// Close the session with [`App::desktop_close`](crate::App::desktop_close).
#[derive(Clone)]
pub struct VncTarget {
    pub id: u32,
    pub conn: Arc<SshConnection>,
    pub vnc_port: u16,
    /// One-time VNC password for this session only (x11vnc exits after it).
    pub password: String,
    pub display: String,
    pub desktop: Option<String>,
}

pub(crate) async fn desktop_open(app: &Arc<App>, host_id: String) -> CmdResult<VncTarget> {
    let conn = app.state.connection(&host_id).await?;
    let VncServer {
        vnc_port,
        password,
        display,
        desktop,
        child_writer,
        mut child_reader,
    } = start_vnc(&conn).await?;
    let id = app.desktops.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    // Keep reading the x11vnc channel so its output never blocks it. When
    // x11vnc exits on its own (its one client left), forget the session.
    let task_app = app.clone();
    let drain = app.rt.spawn(async move {
        while child_reader.next().await.is_some() {}
        task_app.desktops.sessions.lock().unwrap().remove(&id);
    });
    logging::info(&format!(
        "desktop {id} opened on host {host_id} display {display} (loopback-only port verified)"
    ));
    app.desktops.sessions.lock().unwrap().insert(
        id,
        Session {
            conn: conn.clone(),
            vnc_port,
            child: child_writer,
            drain: drain.abort_handle(),
        },
    );
    Ok(VncTarget {
        id,
        conn,
        vnc_port,
        password,
        display,
        desktop,
    })
}

pub(crate) async fn desktop_close(app: &Arc<App>, id: u32) -> CmdResult<()> {
    let session = app.desktops.sessions.lock().unwrap().remove(&id);
    if let Some(s) = session {
        s.drain.abort();
        let _ = s.child.close().await;
        // Closing the channel does not guarantee x11vnc saw its client leave:
        // stop it explicitly so nothing keeps serving the screen.
        stop_x11vnc(&s.conn, s.vnc_port).await;
        logging::info(&format!("desktop {id} closed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Name=alice\nType=tty\nDisplay=\nActive=yes\nDesktop=\n---\n\
Name=alice\nType=x11\nDisplay=:0\nActive=yes\nDesktop=xfce\n---\n\
Name=other\nType=x11\nDisplay=:1\nActive=yes\nDesktop=kde\n---\n\
@@ME=alice\n@@RUNTIME=/run/user/1000\n@@X11VNC=/usr/bin/x11vnc\n@@XAUTH=/run/lightdm/alice/xauthority\n";

    #[test]
    fn finds_own_active_x11_session() {
        let i = parse_detect(SAMPLE);
        assert_eq!(i.session, "x11");
        assert_eq!(i.display.as_deref(), Some(":0"));
        assert_eq!(i.desktop.as_deref(), Some("xfce"));
        assert_eq!(i.x11vnc.as_deref(), Some("/usr/bin/x11vnc"));
        assert_eq!(i.runtime_dir.as_deref(), Some("/run/user/1000"));
        assert_eq!(
            i.xauthority.as_deref(),
            Some("/run/lightdm/alice/xauthority")
        );
    }

    #[test]
    fn loopback_listener_check() {
        let ok = "LISTEN 0 32 127.0.0.1:43210 0.0.0.0:*\nLISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n";
        assert_eq!(listeners_loopback_only(ok, 43210), Some(true));
        let bad = "LISTEN 0 32 [::]:43210 [::]:*\n";
        assert_eq!(listeners_loopback_only(bad, 43210), Some(false));
        let mixed = "LISTEN 0 32 127.0.0.1:43210 0.0.0.0:*\nLISTEN 0 32 0.0.0.0:43210 0.0.0.0:*\n";
        assert_eq!(listeners_loopback_only(mixed, 43210), Some(false));
        let other = "LISTEN 0 32 127.0.0.1:5900 0.0.0.0:*\n";
        assert_eq!(listeners_loopback_only(other, 43210), None);
        assert!((20000..60000).contains(&random_port("ffffffff")));
    }

    #[test]
    fn text_mode_and_other_users_are_not_shared() {
        let text_only = "Name=me\nType=tty\nDisplay=\nActive=yes\n---\nName=bob\nType=x11\nDisplay=:0\nActive=yes\n---\n@@ME=me\n@@RUNTIME=/run/user/1000\n@@X11VNC=\n";
        let i = parse_detect(text_only);
        assert_eq!(i.session, "none");
        assert_eq!(i.x11vnc, None);
        let wl = "Name=me\nType=wayland\nDisplay=\nActive=yes\nDesktop=gnome\n---\n@@ME=me\n@@RUNTIME=/run/user/1000\n@@X11VNC=\n";
        assert_eq!(parse_detect(wl).session, "wayland");
    }
}
