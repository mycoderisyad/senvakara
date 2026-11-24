//! SFTP helpers (FILE-01, task CORE-06): listing and cancellable transfers
//! with progress. Transfers never overwrite silently and never leave a
//! partial file under the final name (docs/SECURITY.md §7).

mod between;
mod manage;
mod tree;

pub use between::copy_between;
pub use manage::{chmod, remove, rename};
pub use tree::{copy_dir_between, download_dir, upload_dir, walk_local, walk_remote, Copied, Plan};

use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use senvakara_core::{CoreError, ErrorCode};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    /// Unix seconds, if the server reported it.
    pub modified: Option<u32>,
    /// Type and permissions as `ls -l` writes them ("drwxr-xr-x"), if the
    /// server reported them.
    pub permissions: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    /// Canonical absolute path that was listed.
    pub path: String,
    pub entries: Vec<RemoteEntry>,
}

fn err(code: ErrorCode, msg: impl Into<String>) -> anyhow::Error {
    CoreError::new(code, msg).into()
}

/// A remote file name is only safe to use locally if it is a single plain
/// path component (no separators, no `..`, no drive or device tricks).
pub fn safe_local_name(name: &str) -> Option<&str> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':', '\0'])
        || name.ends_with('.')
        || name.ends_with(' ');
    (!bad).then_some(name)
}

/// Join a remote directory and a file name with `/` (remote is POSIX).
pub fn remote_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub async fn list(sftp: &SftpSession, path: &str) -> Result<Listing> {
    let path = sftp
        .canonicalize(path)
        .await
        .context("resolve remote path")?;
    let mut entries: Vec<RemoteEntry> = sftp
        .read_dir(path.clone())
        .await
        .context("read remote directory")?
        .map(|e| {
            let m = e.metadata();
            RemoteEntry {
                name: e.file_name(),
                is_dir: m.is_dir(),
                is_symlink: m.is_symlink(),
                size: m.size.unwrap_or(0),
                modified: m.mtime,
                permissions: m.permissions.map(|_| {
                    let kind = if m.is_dir() {
                        'd'
                    } else if m.is_symlink() {
                        'l'
                    } else {
                        '-'
                    };
                    format!("{kind}{}", m.permissions())
                }),
            }
        })
        .filter(|e| e.name != "." && e.name != "..")
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing { path, entries })
}

/// The last component of a remote path, if it names something a copy can
/// be called (not empty, `.` or `..`).
pub(crate) fn remote_name(path: &str) -> Option<&str> {
    let name = path.trim_end_matches('/').rsplit('/').next()?;
    (!name.is_empty() && name != "." && name != ".." && !name.contains('\0')).then_some(name)
}

/// Create the folder `name` in `dir`. Fails with `Conflict` when something
/// with that name is already there, and with `InvalidInput` when `name` is
/// not a single path component.
pub async fn mkdir(sftp: &SftpSession, dir: &str, name: &str) -> Result<String> {
    if name.contains('/') || remote_name(name).is_none() {
        return Err(err(ErrorCode::InvalidInput, "folder name is not valid"));
    }
    let target = remote_join(dir, name);
    if sftp.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists on the remote host"),
        ));
    }
    sftp.create_dir(target.clone())
        .await
        .context("create remote folder")?;
    Ok(target)
}

/// Download `remote` into `local_dir`. Writes to `<name>.part` and renames on
/// success; removes the partial file on failure or cancellation.
pub async fn download(
    sftp: &SftpSession,
    remote: &str,
    local_dir: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    let name = remote.rsplit('/').next().unwrap_or_default();
    let name = safe_local_name(name).ok_or_else(|| {
        err(
            ErrorCode::InvalidInput,
            "remote file name is not safe to save locally",
        )
    })?;
    let target = local_dir.join(name);
    if target.exists() && !overwrite {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists in the destination folder"),
        ));
    }
    let total = sftp
        .metadata(remote)
        .await
        .context("stat remote file")?
        .size
        .unwrap_or(0);
    let part = local_dir.join(format!("{name}.part"));

    let result = async {
        let mut src = sftp.open(remote).await.context("open remote file")?;
        let mut dst = tokio::fs::File::create(&part)
            .await
            .context("create local file")?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(err(ErrorCode::Cancelled, "transfer cancelled"));
            }
            let n = src.read(&mut buf).await.context("read remote file")?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n]).await.context("write local file")?;
            done += n as u64;
            progress(done, total);
        }
        dst.flush().await?;
        drop(dst);
        if overwrite && target.exists() {
            tokio::fs::remove_file(&target)
                .await
                .context("replace existing file")?;
        }
        tokio::fs::rename(&part, &target)
            .await
            .context("finalise local file")?;
        anyhow::Ok(())
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result.map(|_| target)
}

/// Upload `local` into `remote_dir`. Writes to `<name>.part` and renames on success, so a
/// cancelled or failed upload never touches a file that was already there; the partial file
/// is removed on failure or cancellation.
///
/// Fails with `Conflict` when the name is taken and `overwrite` is off.
pub async fn upload(
    sftp: &SftpSession,
    local: &Path,
    remote_dir: &str,
    overwrite: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<String> {
    let name = local.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        err(
            ErrorCode::InvalidInput,
            "local file name is not valid UTF-8",
        )
    })?;
    let target = remote_join(remote_dir, name);
    if !overwrite && sftp.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists on the remote host"),
        ));
    }
    let total = tokio::fs::metadata(local)
        .await
        .context("stat local file")?
        .len();

    let part = format!("{target}.part");

    let result = async {
        let mut src = tokio::fs::File::open(local)
            .await
            .context("open local file")?;
        let mut dst = sftp
            .open_with_flags(
                part.clone(),
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .context("create remote file")?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, total);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(err(ErrorCode::Cancelled, "transfer cancelled"));
            }
            let n = src.read(&mut buf).await.context("read local file")?;
            if n == 0 {
                break;
            }
            dst.write_all(&buf[..n])
                .await
                .context("write remote file")?;
            done += n as u64;
            progress(done, total);
        }
        dst.shutdown().await.context("close remote file")?;
        if done != total {
            return Err(err(
                ErrorCode::InternalError,
                format!("size mismatch after upload ({done} vs {total} bytes)"),
            ));
        }
        if overwrite {
            let _ = sftp.remove_file(target.clone()).await;
        }
        sftp.rename(part.clone(), target.clone())
            .await
            .context("finalise remote file")?;
        anyhow::Ok(())
    }
    .await;

    if result.is_err() {
        let _ = sftp.remove_file(part).await;
    }
    result.map(|_| target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_local_names() {
        for bad in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            r"..\evil.exe",
            "C:x",
            "trail.",
            "trail ",
        ] {
            assert!(safe_local_name(bad).is_none(), "{bad:?} should be rejected");
        }
        assert_eq!(safe_local_name("app-release.apk"), Some("app-release.apk"));
    }

    #[test]
    fn joins_remote_paths() {
        assert_eq!(remote_join("/home/u", "f"), "/home/u/f");
        assert_eq!(remote_join("/", "f"), "/f");
    }
}
