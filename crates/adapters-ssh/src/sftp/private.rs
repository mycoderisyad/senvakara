//! Private run folders for workflow scripts (WF-SPIKE-02, ADR-030): a folder
//! only its owner can read, and files created exclusively with their mode set
//! at open time, so a script is never readable by others for even a moment.

use super::chmod;
use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use senvakara_core::{CoreError, ErrorCode};
use tokio::io::AsyncWriteExt;

fn checked(path: &str) -> Result<()> {
    if !path.starts_with('/') || path.contains('\0') || path.split('/').any(|p| p == "..") {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "a remote run path must be absolute, without '..'",
        )
        .into());
    }
    Ok(())
}

/// Creates `path` and any missing parent below `/` with `mode`, then sets
/// `mode` on `path` itself even when it already existed.
///
/// Fails with `InvalidInput` for a relative path or one with `..`.
pub async fn ensure_private_dir(sftp: &SftpSession, path: &str, mode: u32) -> Result<()> {
    checked(path)?;
    let mut current = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        current.push('/');
        current.push_str(part);
        if !sftp
            .try_exists(current.as_str())
            .await
            .context("check remote folder")?
        {
            sftp.create_dir(current.as_str())
                .await
                .context("create remote folder")?;
            chmod(sftp, &current, mode).await?;
        }
    }
    chmod(sftp, path, mode).await
}

/// Writes `bytes` to a new file at `path` with `mode` set when it is created.
/// Never replaces or follows an existing file: an existing path is an error.
///
/// Fails with `InvalidInput` for a relative path or one with `..`.
pub async fn upload_bytes(sftp: &SftpSession, path: &str, bytes: &[u8], mode: u32) -> Result<()> {
    checked(path)?;
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(mode);
    let mut file = sftp
        .open_with_flags_and_attributes(
            path.to_string(),
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            attrs,
        )
        .await
        .context("create remote file")?;
    file.write_all(bytes).await.context("write remote file")?;
    file.shutdown().await.context("close remote file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::checked;

    #[test]
    fn run_paths_must_be_absolute_and_stay_put() {
        assert!(checked("/home/dev/.cache/senvakara/run-1/n1.sh").is_ok());
        assert!(checked("relative/run").is_err());
        assert!(checked("/home/dev/../root/x").is_err());
        assert!(checked("/home/dev/a\0b").is_err());
    }
}
