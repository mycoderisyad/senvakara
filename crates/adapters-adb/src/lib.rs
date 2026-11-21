//! Minimal ADB host-protocol client (AND-01..04, tasks AND-IMPL-01..06).
//!
//! Talks to an ADB server over TCP: either the local Windows server on
//! `127.0.0.1:5037`, or a Linux host's server reached through an SSH local
//! port-forward (ADR-014). Using the wire protocol directly (instead of
//! spawning `adb.exe -H ...`) means a mismatched client can never kill a
//! remote server, and device changes stream in without polling.
//!
//! Every device operation is bound to a server *and* a transport id. ADB
//! assigns a new transport id when a device reconnects, so a stale handle
//! fails instead of silently reaching a different device (docs/SECURITY.md §2).

pub mod protocol;
pub mod sync;

pub use protocol::{parse_devices_l, AdbDevice};

use anyhow::{bail, Context, Result};
use senvakara_core::{CoreError, ErrorCode};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const IO_TIMEOUT: Duration = Duration::from_secs(10);

fn core(code: ErrorCode, msg: impl Into<String>) -> anyhow::Error {
    CoreError::new(code, msg).into()
}

/// One ADB server endpoint as seen from this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdbServer {
    pub addr: SocketAddr,
}

impl AdbServer {
    pub fn local_default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 5037)),
        }
    }

    async fn connect(&self) -> Result<TcpStream> {
        match tokio::time::timeout(IO_TIMEOUT, TcpStream::connect(self.addr)).await {
            Ok(Ok(s)) => Ok(s),
            Ok(Err(e)) => Err(core(
                ErrorCode::HostOffline,
                format!("ADB server at {} not reachable: {e}", self.addr),
            )),
            Err(_) => Err(core(
                ErrorCode::HostOffline,
                format!("ADB server at {} timed out", self.addr),
            )),
        }
    }

    /// `host:version` — the server's protocol version number.
    pub async fn version(&self) -> Result<u32> {
        let mut s = self.connect().await?;
        protocol::request(&mut s, "host:version").await?;
        let body = protocol::read_hex_block(&mut s).await?;
        u32::from_str_radix(body.trim(), 16).context("parse adb version")
    }

    /// `host:devices-l` — current devices with state and transport ids.
    pub async fn devices(&self) -> Result<Vec<AdbDevice>> {
        let mut s = self.connect().await?;
        protocol::request(&mut s, "host:devices-l").await?;
        Ok(parse_devices_l(&protocol::read_hex_block(&mut s).await?))
    }

    /// `host:track-devices-l` — a stream that yields the full device list
    /// every time it changes. Ends when the server or tunnel goes away.
    pub async fn track(&self) -> Result<DeviceTracker> {
        let mut s = self.connect().await?;
        protocol::request(&mut s, "host:track-devices-l").await?;
        Ok(DeviceTracker { stream: s })
    }

    /// Run a shell command on exactly one transport and return its output.
    /// Output is capped at `max_bytes`; the call is bounded by `timeout`.
    pub async fn shell(
        &self,
        transport_id: u64,
        command: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<ShellOutput> {
        if command.trim().is_empty() {
            bail!(core(ErrorCode::InvalidInput, "empty shell command"));
        }
        let mut s = self
            .open_service(transport_id, &format!("shell:{command}"))
            .await?;
        let (out, truncated) = read_bounded(&mut s, timeout, max_bytes).await?;
        Ok(ShellOutput {
            output: String::from_utf8_lossy(&out).into_owned(),
            truncated,
        })
    }

    /// Bind a fresh connection to exactly one transport and open `service`
    /// on it. A transport that no longer exists fails as `ROUTE_STALE`.
    pub async fn open_service(&self, transport_id: u64, service: &str) -> Result<TcpStream> {
        let mut s = self.connect().await?;
        protocol::request(&mut s, &format!("host:transport-id:{transport_id}"))
            .await
            .map_err(|e| {
                core(
                    ErrorCode::RouteStale,
                    format!("transport {transport_id} is gone: {e}"),
                )
            })?;
        protocol::request(&mut s, service).await?;
        Ok(s)
    }

    /// `exec:` runs a command without a PTY and returns raw, binary-safe
    /// stdout (used for `screencap -p`).
    pub async fn exec_bytes(
        &self,
        transport_id: u64,
        command: &str,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        let mut s = self
            .open_service(transport_id, &format!("exec:{command}"))
            .await?;
        let (out, truncated) = read_bounded(&mut s, timeout, max_bytes).await?;
        if truncated {
            bail!(core(
                ErrorCode::InternalError,
                format!("output larger than {max_bytes} bytes")
            ));
        }
        Ok(out)
    }

    /// Open the file-sync service on one transport (see [`sync`]).
    pub async fn sync(&self, transport_id: u64) -> Result<TcpStream> {
        self.open_service(transport_id, "sync:").await
    }

    /// `getprop <name>` on one transport.
    pub async fn getprop(&self, transport_id: u64, name: &str) -> Result<String> {
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            bail!(core(ErrorCode::InvalidInput, "invalid property name"));
        }
        let out = self
            .shell(transport_id, &format!("getprop {name}"), IO_TIMEOUT, 4096)
            .await?;
        Ok(out.output.trim().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub output: String,
    pub truncated: bool,
}

pub struct DeviceTracker {
    stream: TcpStream,
}

impl DeviceTracker {
    /// Next full snapshot of the device list. `Err` when the stream ends.
    pub async fn next(&mut self) -> Result<Vec<AdbDevice>> {
        Ok(parse_devices_l(
            &protocol::read_hex_block(&mut self.stream).await?,
        ))
    }
}

/// Locate `adb.exe` without bundling it (ADR-015): PATH, then the Android SDK
/// locations Android Studio and the SDK manager use.
pub fn find_local_adb() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut candidates: Vec<PathBuf> = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(v) = std::env::var_os(var) {
            candidates.push(PathBuf::from(v).join("platform-tools").join(exe));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join(exe),
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|p| p.join(exe)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Start the local ADB server (binds 127.0.0.1:5037 by default) if needed.
pub async fn ensure_local_server(adb: &std::path::Path) -> Result<AdbServer> {
    let server = AdbServer::local_default();
    if server.version().await.is_ok() {
        return Ok(server);
    }
    let status = tokio::process::Command::new(adb)
        .arg("start-server")
        .kill_on_drop(true)
        .status()
        .await
        .context("run adb start-server")?;
    if !status.success() {
        bail!(core(ErrorCode::InternalError, "adb start-server failed"));
    }
    server.version().await.map(|_| server)
}

/// Read a stream to EOF with a byte cap and an overall timeout.
async fn read_bounded(
    s: &mut TcpStream,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut truncated = false;
    let read_all = async {
        loop {
            let n = s.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            let room = max_bytes.saturating_sub(out.len());
            out.extend_from_slice(&buf[..n.min(room)]);
            if n > room {
                truncated = true;
                break;
            }
        }
        anyhow::Ok(())
    };
    match tokio::time::timeout(timeout, read_all).await {
        Ok(r) => r?,
        Err(_) => {
            return Err(core(
                ErrorCode::Timeout,
                format!("command did not finish within {timeout:?}"),
            ))
        }
    }
    Ok((out, truncated))
}
