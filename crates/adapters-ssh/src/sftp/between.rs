//! Host to host (FILE-H2H-01): stream one file from an SFTP session to
//! another through this PC, without a temporary copy on it.

use super::{err, remote_join, remote_name, CHUNK};
use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use senvakara_core::ErrorCode;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Copy the file `source` of one host into the folder `dest_dir` of another.
/// Writes to `<name>.part` and renames on success; the partial file is
/// removed on failure or cancellation. `same_host` says both sessions reach
/// one machine: copying a file onto itself is then refused.
///
/// Fails with `Conflict` when the name is taken and `overwrite` is off.
#[allow(clippy::too_many_arguments)] // one transfer, fully described
pub async fn copy_between(
    from: &SftpSession,
    source: &str,
    to: &SftpSession,
    dest_dir: &str,
    overwrite: bool,
    same_host: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<String> {
    let name = remote_name(source)
        .ok_or_else(|| err(ErrorCode::InvalidInput, "file name is not supported"))?;
    let target = remote_join(dest_dir, name);
    if same_host {
        let here = from
            .canonicalize(source)
            .await
            .context("resolve source path")?;
        let there = to
            .canonicalize(dest_dir)
            .await
            .context("resolve destination folder")?;
        if remote_join(&there, name) == here {
            return Err(err(
                ErrorCode::InvalidInput,
                "the file is already in this folder",
            ));
        }
    }
    if !overwrite && to.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists on the remote host"),
        ));
    }
    let total = from
        .metadata(source)
        .await
        .context("stat source file")?
        .size
        .unwrap_or(0);
    let part = format!("{target}.part");

    let result = async {
        let mut src = from.open(source).await.context("open source file")?;
        let mut dst = to
            .open_with_flags(
                part.clone(),
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .context("create destination file")?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(err(ErrorCode::Cancelled, "transfer cancelled"));
            }
            let n = src.read(&mut buf).await.context("read source file")?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n])
                .await
                .context("write destination file")?;
            done += n as u64;
            progress(done, total);
        }
        dst.shutdown().await.context("close destination file")?;
        if done != total {
            return Err(err(
                ErrorCode::InternalError,
                format!("size mismatch after copy ({done} vs {total} bytes)"),
            ));
        }
        if overwrite {
            let _ = to.remove_file(target.clone()).await;
        }
        to.rename(part.clone(), target.clone())
            .await
            .context("finalise destination file")?;
        anyhow::Ok(())
    }
    .await;

    if result.is_err() {
        let _ = to.remove_file(part).await;
    }
    result.map(|_| target)
}
