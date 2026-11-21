//! ADB file sync protocol (`sync:` service) for push/pull/list/stat
//! (AND-07, FILE-02, task AND-IMPL-10).
//!
//! After `sync:` is accepted, each message is a 4-byte ASCII id followed by a
//! little-endian u32: `STAT`/`LIST`/`SEND`/`RECV` + path length + path,
//! `DATA` + length + bytes (≤ 64 KiB), `DONE` + mtime, `OKAY`/`FAIL`.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_DATA: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteStat {
    pub mode: u32,
    pub size: u32,
    pub mtime: u32,
}

impl RemoteStat {
    pub fn exists(&self) -> bool {
        self.mode != 0
    }
    pub fn is_dir(&self) -> bool {
        self.mode & 0o170000 == 0o040000
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Legacy sync reports 32-bit sizes.
    pub size: u32,
    pub mtime: u32,
}

async fn send_request<S: AsyncWrite + Unpin>(s: &mut S, id: &[u8; 4], path: &str) -> Result<()> {
    if path.len() > 1024 {
        bail!("path too long");
    }
    s.write_all(id).await?;
    s.write_all(&(path.len() as u32).to_le_bytes()).await?;
    s.write_all(path.as_bytes()).await?;
    Ok(())
}

async fn read_u32<S: AsyncRead + Unpin>(s: &mut S) -> Result<u32> {
    Ok(s.read_u32_le().await?)
}

async fn read_id<S: AsyncRead + Unpin>(s: &mut S) -> Result<[u8; 4]> {
    let mut id = [0u8; 4];
    s.read_exact(&mut id).await.context("sync stream closed")?;
    Ok(id)
}

async fn read_fail<S: AsyncRead + Unpin>(s: &mut S) -> anyhow::Error {
    let len = match read_u32(s).await {
        Ok(n) => n.min(4096) as usize,
        Err(e) => return e,
    };
    let mut msg = vec![0u8; len];
    if let Err(e) = s.read_exact(&mut msg).await {
        return e.into();
    }
    anyhow::anyhow!("device refused: {}", String::from_utf8_lossy(&msg))
}

pub async fn stat<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, path: &str) -> Result<RemoteStat> {
    send_request(s, b"STAT", path).await?;
    if &read_id(s).await? != b"STAT" {
        bail!("unexpected reply to STAT");
    }
    Ok(RemoteStat {
        mode: read_u32(s).await?,
        size: read_u32(s).await?,
        mtime: read_u32(s).await?,
    })
}

pub async fn list<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    path: &str,
) -> Result<Vec<SyncEntry>> {
    send_request(s, b"LIST", path).await?;
    let mut out = Vec::new();
    loop {
        match &read_id(s).await? {
            b"DENT" => {
                let mode = read_u32(s).await?;
                let size = read_u32(s).await?;
                let mtime = read_u32(s).await?;
                let len = read_u32(s).await? as usize;
                if len > 4096 {
                    bail!("directory entry name too long");
                }
                let mut name = vec![0u8; len];
                s.read_exact(&mut name).await?;
                let name = String::from_utf8_lossy(&name).into_owned();
                if name != "." && name != ".." {
                    out.push(SyncEntry {
                        name,
                        is_dir: mode & 0o170000 == 0o040000,
                        is_symlink: mode & 0o170000 == 0o120000,
                        size,
                        mtime,
                    });
                }
            }
            b"DONE" => {
                // DONE carries a dummy entry (mode, size, time, namelen).
                for _ in 0..4 {
                    read_u32(s).await?;
                }
                break;
            }
            b"FAIL" => return Err(read_fail(s).await),
            other => bail!("unexpected LIST reply {:?}", String::from_utf8_lossy(other)),
        }
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// Upload `src` to `path` on the device with permission `mode` (e.g. 0o644).
pub async fn send<S, R>(
    s: &mut S,
    src: &mut R,
    path: &str,
    mode: u32,
    mtime: u32,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    send_request(s, b"SEND", &format!("{path},{mode}")).await?;
    let mut buf = vec![0u8; MAX_DATA];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            // Abandon the connection; the device discards the partial file
            // only on some versions, so callers delete it afterwards.
            bail!(senvakara_core::CoreError::new(
                senvakara_core::ErrorCode::Cancelled,
                "transfer cancelled"
            ));
        }
        let n = src.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        s.write_all(b"DATA").await?;
        s.write_all(&(n as u32).to_le_bytes()).await?;
        s.write_all(&buf[..n]).await?;
        done += n as u64;
        progress(done);
    }
    s.write_all(b"DONE").await?;
    s.write_all(&mtime.to_le_bytes()).await?;
    match &read_id(s).await? {
        b"OKAY" => {
            read_u32(s).await?;
            Ok(())
        }
        b"FAIL" => Err(read_fail(s).await),
        other => bail!("unexpected SEND reply {:?}", String::from_utf8_lossy(other)),
    }
}

/// Download `path` from the device into `dst`.
pub async fn recv<S, W>(
    s: &mut S,
    path: &str,
    dst: &mut W,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<u64>
where
    S: AsyncRead + AsyncWrite + Unpin,
    W: AsyncWrite + Unpin,
{
    send_request(s, b"RECV", path).await?;
    let mut buf = vec![0u8; MAX_DATA];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!(senvakara_core::CoreError::new(
                senvakara_core::ErrorCode::Cancelled,
                "transfer cancelled"
            ));
        }
        match &read_id(s).await? {
            b"DATA" => {
                let n = read_u32(s).await? as usize;
                if n > MAX_DATA {
                    bail!("sync DATA chunk too large");
                }
                s.read_exact(&mut buf[..n]).await?;
                dst.write_all(&buf[..n]).await?;
                done += n as u64;
                progress(done);
            }
            b"DONE" => {
                read_u32(s).await?;
                dst.flush().await?;
                return Ok(done);
            }
            b"FAIL" => return Err(read_fail(s).await),
            other => bail!("unexpected RECV reply {:?}", String::from_utf8_lossy(other)),
        }
    }
}

pub async fn quit<S: AsyncWrite + Unpin>(s: &mut S) {
    let _ = s.write_all(b"QUIT\0\0\0\0").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_parses_dents_and_done() {
        let (mut client, mut device) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut req = vec![0u8; 8 + 4];
            device.read_exact(&mut req).await.unwrap();
            let mut reply = Vec::new();
            for (mode, size, name) in [
                (0o040755u32, 0u32, "Download"),
                (0o100644, 12, "a.txt"),
                (0o040755, 0, "."),
            ] {
                reply.extend_from_slice(b"DENT");
                for v in [mode, size, 1_700_000_000, name.len() as u32] {
                    reply.extend_from_slice(&v.to_le_bytes());
                }
                reply.extend_from_slice(name.as_bytes());
            }
            reply.extend_from_slice(b"DONE");
            reply.extend_from_slice(&[0u8; 16]);
            device.write_all(&reply).await.unwrap();
        });
        let entries = list(&mut client, "/sdc").await.unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir && entries[0].name == "Download");
        assert_eq!(entries[1].size, 12);
    }

    #[tokio::test]
    async fn recv_reports_device_failure() {
        let (mut client, mut device) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut req = vec![0u8; 8 + 5];
            device.read_exact(&mut req).await.unwrap();
            let msg = b"No such file";
            device.write_all(b"FAIL").await.unwrap();
            device
                .write_all(&(msg.len() as u32).to_le_bytes())
                .await
                .unwrap();
            device.write_all(msg).await.unwrap();
        });
        let mut sink = Vec::new();
        let err = recv(
            &mut client,
            "/nope",
            &mut sink,
            &AtomicBool::new(false),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("No such file"));
    }

    #[test]
    fn stat_mode_helpers() {
        assert!(RemoteStat {
            mode: 0o040755,
            size: 0,
            mtime: 0
        }
        .is_dir());
        assert!(!RemoteStat {
            mode: 0,
            size: 0,
            mtime: 0
        }
        .exists());
    }
}
