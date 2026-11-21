//! Minimal scrcpy client for screen mirroring and input (AND-06,
//! tasks AND-IMPL-08/09). Protocol per scrcpy v3.3.3 `doc/develop.md`,
//! `DesktopConnection.java` and `ControlMessageReader.java`.
//!
//! Flow: push the pinned `scrcpy-server` (SHA-256 checked) → start it with
//! `app_process` over a `shell:` stream → connect twice to
//! `localabstract:scrcpy_<scid>` through the *same* ADB server connection
//! used for the device (so it works for phones behind a Linux host's SSH
//! tunnel too, ADR-014) → read video packets, write control messages.
//! Closing the shell stream stops the server (`cleanup=true`).

pub mod control;
pub mod video;

use anyhow::{bail, Context, Result};
use senvakara_adapters_adb::{sync, AdbServer};
use senvakara_core::{CoreError, ErrorCode};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The server version this client speaks. Client and server must match exactly.
pub const SERVER_VERSION: &str = "3.3.3";
/// SHA-256 of the official `scrcpy-server-v3.3.3` release asset.
pub const SERVER_SHA256: &str = "7e70323ba7f259649dd4acce97ac4fefbae8102b2c6d91e2e7be613fd5354be0";
const DEVICE_JAR: &str = "/data/local/tmp/senvakara-scrcpy-server-v3.3.3.jar";

fn core(code: ErrorCode, msg: impl Into<String>) -> anyhow::Error {
    CoreError::new(code, msg).into()
}

/// Read the bundled server and refuse anything but the pinned build.
pub async fn load_verified_server(path: &Path) -> Result<Vec<u8>> {
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if hex != SERVER_SHA256 {
        bail!(core(
            ErrorCode::InternalError,
            "bundled scrcpy-server failed its SHA-256 check"
        ));
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct MirrorOptions {
    /// Longest side in pixels (multiple of 8); 0 = device resolution.
    pub max_size: u32,
    pub max_fps: u32,
    pub video_bit_rate: u32,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self {
            max_size: 1280,
            max_fps: 30,
            video_bit_rate: 4_000_000,
        }
    }
}

pub struct Mirror {
    pub device_name: String,
    pub codec_id: u32,
    pub width: u32,
    pub height: u32,
    pub video: video::VideoReader,
    pub control: control::ControlWriter,
    /// Device→client messages (clipboard etc.); must be drained.
    pub control_rx: tokio::net::tcp::OwnedReadHalf,
    /// Keeps the server process alive; dropping it stops the server.
    pub server_shell: TcpStream,
}

/// Start mirroring on one transport of `server`.
pub async fn start(
    server: AdbServer,
    transport_id: u64,
    jar: &[u8],
    opts: &MirrorOptions,
) -> Result<Mirror> {
    // 1. Push the server unless an identical copy is already there.
    let mut s = server.sync(transport_id).await?;
    let st = sync::stat(&mut s, DEVICE_JAR).await?;
    if st.size as usize != jar.len() {
        let mut src = jar;
        sync::send(
            &mut s,
            &mut src,
            DEVICE_JAR,
            0o644,
            0,
            &AtomicBool::new(false),
            |_| {},
        )
        .await?;
    }
    sync::quit(&mut s).await;

    // 2. Start it. Every option value below is generated here, never user text.
    let scid = random_scid();
    let cmd = format!(
        "shell:CLASSPATH={DEVICE_JAR} app_process / com.genymobile.scrcpy.Server {SERVER_VERSION} \
         scid={scid:08x} log_level=info audio=false control=true tunnel_forward=true cleanup=true \
         max_size={} max_fps={} video_bit_rate={} power_off_on_close=false",
        opts.max_size & !7,
        opts.max_fps,
        opts.video_bit_rate,
    );
    let mut server_shell = server.open_service(transport_id, &cmd).await?;
    let socket = format!("localabstract:scrcpy_{scid:08x}");

    // 3. Video socket: retry until the server listens; the dummy byte proves it.
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut video = loop {
        if Instant::now() > deadline {
            let mut log = vec![0u8; 4096];
            let n = tokio::time::timeout(Duration::from_millis(300), server_shell.read(&mut log))
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(0);
            bail!(core(
                ErrorCode::Timeout,
                format!(
                    "scrcpy server did not start: {}",
                    String::from_utf8_lossy(&log[..n]).trim()
                )
            ));
        }
        if let Ok(mut v) = server.open_service(transport_id, &socket).await {
            let mut dummy = [0u8; 1];
            if let Ok(Ok(1)) =
                tokio::time::timeout(Duration::from_secs(2), v.read(&mut dummy)).await
            {
                break v;
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    };
    // 4. Control socket is the server's second accept().
    let control = server
        .open_service(transport_id, &socket)
        .await
        .context("open control socket")?;

    // 5. Device meta (64 bytes, NUL-padded) then codec meta (3 × u32 BE).
    let mut name = [0u8; 64];
    video.read_exact(&mut name).await?;
    let device_name = String::from_utf8_lossy(&name)
        .trim_end_matches('\0')
        .to_string();
    let codec_id = video.read_u32().await?;
    let width = video.read_u32().await?;
    let height = video.read_u32().await?;
    if codec_id != video::CODEC_H264 {
        bail!(core(
            ErrorCode::UnsupportedCapability,
            format!("unexpected video codec 0x{codec_id:08x}")
        ));
    }
    let (control_rx, control_tx) = control.into_split();
    Ok(Mirror {
        device_name,
        codec_id,
        width,
        height,
        video: video::VideoReader::new(video),
        control: control::ControlWriter::new(control_tx),
        control_rx,
        server_shell,
    })
}

fn random_scid() -> u32 {
    // 31-bit id, unique enough to keep concurrent sessions on one device apart.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(nanos.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let d = h.finalize();
    u32::from_be_bytes([d[0], d[1], d[2], d[3]]) & 0x7fff_ffff
}

/// Gracefully end a session: closing the shell stream stops the server.
pub async fn stop(mut m: Mirror) {
    let _ = m.server_shell.shutdown().await;
}
