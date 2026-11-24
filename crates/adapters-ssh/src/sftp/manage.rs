//! Remote SFTP item management: rename, recursive delete without following
//! symlinks, and permission updates (`chmod`).

use super::tree::{too_large, MAX_ENTRIES};
use super::{err, remote_join, remote_name};
use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use senvakara_core::ErrorCode;

fn valid_component(name: &str) -> bool {
    !name.contains('/') && remote_name(name).is_some()
}

fn validate_rename(dir: &str, old_name: &str, new_name: &str) -> Result<()> {
    if dir.trim().is_empty() || dir.contains('\0') {
        return Err(err(ErrorCode::InvalidInput, "folder path is not valid"));
    }
    if !valid_component(old_name) || !valid_component(new_name) {
        return Err(err(ErrorCode::InvalidInput, "item name is not valid"));
    }
    Ok(())
}

fn clean_remove_path(path: &str) -> Result<&str> {
    let path = path.trim();
    if path.is_empty() || path.contains('\0') {
        return Err(err(ErrorCode::InvalidInput, "remote path is not valid"));
    }
    // Strip trailing slashes so `lstat` never resolves a symlink to a folder.
    let path = path.trim_end_matches('/');
    if path.is_empty() || remote_name(path).is_none() {
        return Err(err(ErrorCode::InvalidInput, "remote path is not valid"));
    }
    Ok(path)
}

fn validate_chmod(path: &str, mode: u32) -> Result<()> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err(err(ErrorCode::InvalidInput, "remote path is not valid"));
    }
    if mode > 0o7777 {
        return Err(err(
            ErrorCode::InvalidInput,
            "permissions mode must be between 0000 and 7777",
        ));
    }
    Ok(())
}

/// Rename `old_name` in `dir` to `new_name` and return the resulting path.
///
/// Fails with `InvalidInput` when `dir` is invalid or either name is not a
/// single POSIX path component, and with `Conflict` when `new_name` already
/// exists (including broken symlinks).
pub async fn rename(
    sftp: &SftpSession,
    dir: &str,
    old_name: &str,
    new_name: &str,
) -> Result<String> {
    validate_rename(dir, old_name, new_name)?;
    if old_name == new_name {
        return Ok(remote_join(dir, new_name));
    }
    let from = remote_join(dir, old_name);
    let to = remote_join(dir, new_name);
    if sftp.symlink_metadata(to.clone()).await.is_ok() {
        return Err(err(
            ErrorCode::Conflict,
            format!("{new_name} already exists on the remote host"),
        ));
    }
    sftp.rename(from, to.clone())
        .await
        .context("rename remote item")?;
    Ok(to)
}

/// Remove a remote file, symlink, or folder tree without following symlinks.
///
/// Fails with `InvalidInput` when `path` is empty, `.`, `..`, `/` (or all
/// slashes), contains `\0`, or the tree holds more than 200,000 items or an
/// invalid entry name.
pub async fn remove(sftp: &SftpSession, path: &str) -> Result<()> {
    let path = clean_remove_path(path)?;
    let meta = sftp
        .symlink_metadata(path.to_string())
        .await
        .context("stat remote item")?;
    if !meta.file_type().is_dir() {
        sftp.remove_file(path.to_string())
            .await
            .context("remove remote file")?;
        return Ok(());
    }

    let mut dirs = vec![path.to_string()];
    let mut files = Vec::new();
    let mut pending = vec![path.to_string()];
    let mut count = 0usize;
    while let Some(dir) = pending.pop() {
        let entries = sftp
            .read_dir(dir.clone())
            .await
            .context("read remote directory")?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            if !valid_component(&name) {
                return Err(err(
                    ErrorCode::InvalidInput,
                    "remote folder contains an invalid entry name",
                ));
            }
            count += 1;
            if count > MAX_ENTRIES {
                return Err(too_large());
            }
            let child = remote_join(&dir, &name);
            let is_dir = if entry.metadata().permissions.is_some() {
                entry.file_type().is_dir()
            } else {
                sftp.symlink_metadata(child.clone())
                    .await
                    .context("stat remote item")?
                    .file_type()
                    .is_dir()
            };
            if is_dir {
                dirs.push(child.clone());
                pending.push(child);
            } else {
                files.push(child);
            }
        }
    }

    for file in files {
        sftp.remove_file(file).await.context("remove remote file")?;
    }
    for dir in dirs.into_iter().rev() {
        sftp.remove_dir(dir).await.context("remove remote folder")?;
    }
    Ok(())
}

/// Set POSIX permissions `mode` (`0o000`–`0o7777`) on `path`.
///
/// Fails with `InvalidInput` when `path` is empty or contains `\0`, or when
/// `mode` exceeds `0o7777`.
pub async fn chmod(sftp: &SftpSession, path: &str, mode: u32) -> Result<()> {
    validate_chmod(path, mode)?;
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(mode);
    sftp.set_metadata(path.to_string(), attrs)
        .await
        .context("change remote permissions")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use senvakara_core::CoreError;

    fn code(result: Result<impl std::fmt::Debug>) -> Option<ErrorCode> {
        result
            .unwrap_err()
            .downcast_ref::<CoreError>()
            .map(|error| error.code)
    }

    #[test]
    fn rename_refuses_invalid_names_and_directories() {
        for bad in ["", ".", "..", "/", "a/b", "dir/", "/abs", "../up", "a\0b"] {
            assert_eq!(
                code(validate_rename("/home/me", bad, "good.txt")),
                Some(ErrorCode::InvalidInput),
                "old_name {bad:?}"
            );
            assert_eq!(
                code(validate_rename("/home/me", "good.txt", bad)),
                Some(ErrorCode::InvalidInput),
                "new_name {bad:?}"
            );
        }
        for bad_dir in ["", "   ", "/home\0me"] {
            assert_eq!(
                code(validate_rename(bad_dir, "old.txt", "new.txt")),
                Some(ErrorCode::InvalidInput),
                "dir {bad_dir:?}"
            );
        }
        assert!(validate_rename("/home/me", "old.txt", ".hidden").is_ok());
        assert!(validate_rename("/", "old.txt", "new name.txt").is_ok());
    }

    #[test]
    fn remove_refuses_empty_root_dot_and_nul_paths_and_strips_trailing_slashes() {
        for bad in [
            "",
            "   ",
            "/",
            "///",
            " / ",
            ".",
            "./",
            "..",
            "../",
            "/.",
            "/..",
            "/home/me/.",
            "/home/me/..",
            "a\0b",
            "/home/me/a\0b/c",
        ] {
            assert_eq!(
                code(clean_remove_path(bad)),
                Some(ErrorCode::InvalidInput),
                "{bad:?}"
            );
        }
        assert_eq!(
            clean_remove_path(" /home/me/folder/// ").unwrap(),
            "/home/me/folder"
        );
        assert_eq!(clean_remove_path("/a").unwrap(), "/a");
        assert_eq!(clean_remove_path("rel/file.txt").unwrap(), "rel/file.txt");
    }

    #[test]
    fn chmod_validates_path_and_permission_bit_bounds() {
        for bad_path in ["", "   ", "a\0b", "/home/me/\0file"] {
            assert_eq!(
                code(validate_chmod(bad_path, 0o644)),
                Some(ErrorCode::InvalidInput),
                "{bad_path:?}"
            );
        }
        for bad_mode in [0o10000, 0o100644, u32::MAX] {
            assert_eq!(
                code(validate_chmod("/home/me/a.txt", bad_mode)),
                Some(ErrorCode::InvalidInput),
                "{bad_mode:#o}"
            );
        }
        for ok_mode in [0o000, 0o644, 0o755, 0o4755, 0o7777] {
            assert!(
                validate_chmod("/home/me/a.txt", ok_mode).is_ok(),
                "{ok_mode:#o}"
            );
        }
    }
}
