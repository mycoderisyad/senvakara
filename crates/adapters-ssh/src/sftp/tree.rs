//! Folders (FILE-DIR-01): find what a folder holds, then create the folders
//! and copy the files one by one, reporting progress over the whole tree.
//! Links are never followed: they are counted as skipped, so a tree cannot
//! loop and nothing outside it is copied.

use super::{copy_between, download, err, remote_join, remote_name, safe_local_name, upload};
use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use senvakara_core::ErrorCode;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// More entries than this in one folder tree is refused, so a host cannot
/// make this PC collect an endless listing.
const MAX_ENTRIES: usize = 200_000;

/// What a folder holds below itself, found before anything is copied. Paths
/// are relative to the folder and use `/`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Folders to create, parents before their children.
    pub dirs: Vec<String>,
    /// Files with their size in bytes.
    pub files: Vec<(String, u64)>,
    /// Links, special files and names that cannot be stored: left out.
    pub skipped: usize,
}

impl Plan {
    pub fn total(&self) -> u64 {
        self.files.iter().map(|(_, size)| size).sum()
    }

    fn entries(&self) -> usize {
        self.dirs.len() + self.files.len()
    }
}

/// A finished folder copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The folder that was written.
    pub path: String,
    /// How many links and unsupported entries were left out.
    pub skipped: usize,
}

impl Copied {
    /// The result as the job list shows it.
    pub fn describe(&self) -> String {
        match self.skipped {
            0 => self.path.clone(),
            1 => format!("{} (1 link or unsupported item skipped)", self.path),
            n => format!("{} ({n} links or unsupported items skipped)", self.path),
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(err(ErrorCode::Cancelled, "transfer cancelled"));
    }
    Ok(())
}

fn too_large() -> anyhow::Error {
    err(
        ErrorCode::InvalidInput,
        format!("the folder holds more than {MAX_ENTRIES} items"),
    )
}

fn child(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// The folder part of a relative path ("a/b/c.txt" gives "a/b").
fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// `rel` below `root` on this PC. Every component must be a name that is
/// safe to store here, so a path from a host can never leave `root`.
fn local_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split('/').filter(|part| !part.is_empty()) {
        let part = safe_local_name(part).ok_or_else(|| {
            err(
                ErrorCode::InvalidInput,
                "remote file name is not safe to save locally",
            )
        })?;
        path.push(part);
    }
    Ok(path)
}

fn remote_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_string()
    } else {
        remote_join(root, rel)
    }
}

/// List everything below the remote folder `root`. With `for_local`, names
/// that cannot be stored on this PC are skipped too.
pub async fn walk_remote(
    sftp: &SftpSession,
    root: &str,
    for_local: bool,
    cancel: &AtomicBool,
) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        cancelled(cancel)?;
        let entries = sftp
            .read_dir(remote_path(root, &dir))
            .await
            .context("read remote directory")?;
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let kind = entry.file_type();
            let storable = !name.contains('/')
                && remote_name(&name).is_some()
                && (!for_local || safe_local_name(&name).is_some());
            if !storable {
                plan.skipped += 1;
            } else if kind.is_dir() {
                let rel = child(&dir, &name);
                plan.dirs.push(rel.clone());
                pending.push(rel);
            } else if kind.is_file() {
                let size = entry.metadata().size.unwrap_or(0);
                plan.files.push((child(&dir, &name), size));
            } else {
                // A link is not followed; devices and sockets are not files.
                plan.skipped += 1;
            }
            if plan.entries() > MAX_ENTRIES {
                return Err(too_large());
            }
        }
    }
    // Parents first, whatever order the folders were visited in.
    plan.dirs.sort();
    Ok(plan)
}

/// List everything below the folder `root` of this PC.
pub async fn walk_local(root: &Path, cancel: &AtomicBool) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut pending = vec![String::new()];
    while let Some(dir) = pending.pop() {
        cancelled(cancel)?;
        let folder = root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR));
        let mut reader = tokio::fs::read_dir(&folder)
            .await
            .context("read local folder")?;
        while let Some(entry) = reader.next_entry().await.context("read local folder")? {
            let kind = entry.file_type().await.context("read local folder")?;
            let name = entry.file_name().into_string().ok();
            match name {
                // A link (or a junction) is not followed.
                Some(name) if !kind.is_symlink() && !name.contains('/') => {
                    let rel = child(&dir, &name);
                    if kind.is_dir() {
                        plan.dirs.push(rel.clone());
                        pending.push(rel);
                    } else {
                        let size = entry.metadata().await.map(|meta| meta.len()).unwrap_or(0);
                        plan.files.push((rel, size));
                    }
                }
                _ => plan.skipped += 1,
            }
            if plan.entries() > MAX_ENTRIES {
                return Err(too_large());
            }
        }
    }
    plan.dirs.sort();
    Ok(plan)
}

/// Make sure the remote folder `path` exists.
async fn ensure_remote_dir(sftp: &SftpSession, path: &str) -> Result<()> {
    if sftp.create_dir(path).await.is_ok() {
        return Ok(());
    }
    let meta = sftp.metadata(path).await.context("create remote folder")?;
    if meta.is_dir() {
        Ok(())
    } else {
        Err(err(
            ErrorCode::Conflict,
            "a file with the folder's name is in the way",
        ))
    }
}

// In the three copies below, an error names the file it happened on.

/// Download the remote folder `remote` and everything in it into
/// `local_dir`. An existing folder of that name is a `Conflict` unless
/// `overwrite` is set; then the two are merged and files of the same name
/// replaced. Cancelling stops between files and removes the partial one.
pub async fn download_dir(
    sftp: &SftpSession,
    remote: &str,
    local_dir: &Path,
    overwrite: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Copied> {
    let remote = remote.trim_end_matches('/');
    let name = remote_name(remote)
        .and_then(safe_local_name)
        .ok_or_else(|| {
            err(
                ErrorCode::InvalidInput,
                "remote folder name is not safe to save locally",
            )
        })?;
    let target = local_dir.join(name);
    if target.exists() && !overwrite {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists in the destination folder"),
        ));
    }
    let plan = walk_remote(sftp, remote, true, cancel).await?;
    let total = plan.total();
    progress(0, total);
    tokio::fs::create_dir_all(&target)
        .await
        .context("create local folder")?;
    for dir in &plan.dirs {
        tokio::fs::create_dir_all(local_path(&target, dir)?)
            .await
            .context("create local folder")?;
    }
    let mut base = 0u64;
    for (file, size) in &plan.files {
        cancelled(cancel)?;
        download(
            sftp,
            &remote_join(remote, file),
            &local_path(&target, parent_of(file))?,
            true,
            cancel,
            |done, _| progress(base + done, total),
        )
        .await
        .with_context(|| format!("copy {file}"))?;
        base += size;
    }
    Ok(Copied {
        path: target.display().to_string(),
        skipped: plan.skipped,
    })
}

/// Upload the folder `local` and everything in it into `remote_dir`. Same
/// rules as [`download_dir`].
pub async fn upload_dir(
    sftp: &SftpSession,
    local: &Path,
    remote_dir: &str,
    overwrite: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Copied> {
    let name = local
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            err(
                ErrorCode::InvalidInput,
                "local folder name is not valid UTF-8",
            )
        })?;
    let target = remote_join(remote_dir, name);
    if !overwrite && sftp.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists on the remote host"),
        ));
    }
    let plan = walk_local(local, cancel).await?;
    let total = plan.total();
    progress(0, total);
    ensure_remote_dir(sftp, &target).await?;
    for dir in &plan.dirs {
        ensure_remote_dir(sftp, &remote_join(&target, dir)).await?;
    }
    let mut base = 0u64;
    for (file, size) in &plan.files {
        cancelled(cancel)?;
        upload(
            sftp,
            &local.join(file.replace('/', std::path::MAIN_SEPARATOR_STR)),
            &remote_path(&target, parent_of(file)),
            true,
            cancel,
            |done, _| progress(base + done, total),
        )
        .await
        .with_context(|| format!("copy {file}"))?;
        base += size;
    }
    Ok(Copied {
        path: target,
        skipped: plan.skipped,
    })
}

/// Copy the folder `source` of one host into the folder `dest_dir` of
/// another, through this PC. Same rules as [`download_dir`]. `same_host`
/// says both sessions reach one machine: a folder is then never copied onto
/// or into itself.
#[allow(clippy::too_many_arguments)] // one transfer, fully described
pub async fn copy_dir_between(
    from: &SftpSession,
    source: &str,
    to: &SftpSession,
    dest_dir: &str,
    overwrite: bool,
    same_host: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Copied> {
    let source = source.trim_end_matches('/');
    let name = remote_name(source)
        .ok_or_else(|| err(ErrorCode::InvalidInput, "folder name is not supported"))?;
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
        if inside(&here, &there) {
            return Err(err(
                ErrorCode::InvalidInput,
                "a folder cannot be copied into itself",
            ));
        }
        if remote_join(&there, name) == here {
            return Err(err(
                ErrorCode::InvalidInput,
                "the folder is already in this place",
            ));
        }
    }
    if !overwrite && to.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(err(
            ErrorCode::Conflict,
            format!("{name} already exists on the remote host"),
        ));
    }
    let plan = walk_remote(from, source, false, cancel).await?;
    let total = plan.total();
    progress(0, total);
    ensure_remote_dir(to, &target).await?;
    for dir in &plan.dirs {
        ensure_remote_dir(to, &remote_join(&target, dir)).await?;
    }
    let mut base = 0u64;
    for (file, size) in &plan.files {
        cancelled(cancel)?;
        copy_between(
            from,
            &remote_join(source, file),
            to,
            &remote_path(&target, parent_of(file)),
            true,
            false,
            cancel,
            |done, _| progress(base + done, total),
        )
        .await
        .with_context(|| format!("copy {file}"))?;
        base += size;
    }
    Ok(Copied {
        path: target,
        skipped: plan.skipped,
    })
}

/// Whether the folder `inner` is `outer` or lies below it.
fn inside(outer: &str, inner: &str) -> bool {
    let outer = outer.trim_end_matches('/');
    inner == outer
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_path_can_never_leave_the_local_folder() {
        let root = Path::new("C:\\Downloads\\site");
        assert_eq!(
            local_path(root, "css/main.css").unwrap(),
            root.join("css").join("main.css")
        );
        assert_eq!(local_path(root, "").unwrap(), root);
        for bad in ["../evil", "a/../../evil", "a/C:evil", "a\\..\\evil", "nul."] {
            assert!(local_path(root, bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_folder_is_inside_itself_and_its_children_only() {
        assert!(inside("/home/me/site", "/home/me/site"));
        assert!(inside("/home/me/site", "/home/me/site/css"));
        assert!(!inside("/home/me/site", "/home/me/site-backup"));
        assert!(!inside("/home/me/site", "/home/me"));
    }

    #[tokio::test]
    async fn a_local_walk_lists_parents_first_and_adds_up_the_sizes() {
        let root = std::env::temp_dir().join(format!("senvakara-walk-{}", std::process::id()));
        std::fs::create_dir_all(root.join("b").join("deep")).unwrap();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("top.txt"), b"12345").unwrap();
        std::fs::write(root.join("b").join("deep").join("x.bin"), b"123").unwrap();
        let plan = walk_local(&root, &AtomicBool::new(false)).await.unwrap();
        assert_eq!(plan.dirs, ["a", "b", "b/deep"]);
        let mut files = plan.files.clone();
        files.sort();
        assert_eq!(
            files,
            [("b/deep/x.bin".to_string(), 3), ("top.txt".to_string(), 5)]
        );
        assert_eq!((plan.total(), plan.skipped), (8, 0));
        // A walk that was cancelled stops with that error.
        let stopped = walk_local(&root, &AtomicBool::new(true)).await.unwrap_err();
        let code = stopped
            .downcast_ref::<senvakara_core::CoreError>()
            .map(|error| error.code);
        assert_eq!(code, Some(ErrorCode::Cancelled));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_result_names_what_was_left_out() {
        let copied = |skipped| Copied {
            path: "/tmp/site".into(),
            skipped,
        };
        assert_eq!(copied(0).describe(), "/tmp/site");
        assert!(copied(1)
            .describe()
            .ends_with("(1 link or unsupported item skipped)"));
        assert!(copied(3).describe().contains("3 links"));
    }
}
