//! Changing what a folder of this PC holds: renaming an item and deleting items. Everything
//! here blocks on the disk: run it off the UI thread.

use std::path::PathBuf;

use super::paths::join;
use super::Kind;

pub(super) const TAKEN: &str = "A file or folder with that name is already here.";

/// Rename `old_name` in `folder` of this PC. Blocking: run it off the UI thread.
pub(super) fn rename_local(folder: &str, old_name: &str, new_name: &str) -> Result<(), String> {
    let from = PathBuf::from(join(Kind::Local, folder, old_name));
    let to = PathBuf::from(join(Kind::Local, folder, new_name));
    // Windows finds the item itself under a name that differs only in case.
    let recased = old_name.eq_ignore_ascii_case(new_name);
    if !recased && to.symlink_metadata().is_ok() {
        return Err(TAKEN.to_string());
    }
    std::fs::rename(&from, &to).map_err(|error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => TAKEN.to_string(),
        std::io::ErrorKind::PermissionDenied => {
            "Windows does not allow renaming this item.".to_string()
        }
        std::io::ErrorKind::NotFound => "This item no longer exists.".to_string(),
        _ => format!("The item could not be renamed. ({error})"),
    })
}

/// Delete `names` in `folder` of this PC, folders with everything in them. A link is removed,
/// never followed. Stops at the first item that fails. Blocking: run it off the UI thread.
pub(super) fn delete_local(folder: &str, names: &[String]) -> Result<(), String> {
    for name in names {
        let target = PathBuf::from(join(Kind::Local, folder, name));
        let meta = std::fs::symlink_metadata(&target)
            .map_err(|error| format!("Could not inspect {name}. ({error})"))?;
        let result = if meta.is_dir() && !meta.file_type().is_symlink() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        result.map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => {
                format!("Windows does not allow deleting {name}.")
            }
            _ => format!("Could not delete {name}. ({error})"),
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_of_this_pc_are_renamed_and_deleted_without_following_into_other_names() {
        let folder = std::env::temp_dir().join(format!("senvakara-manage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("docs")).unwrap();
        std::fs::write(folder.join("docs").join("inner.txt"), "x").unwrap();
        std::fs::write(folder.join("a.txt"), "a").unwrap();
        std::fs::write(folder.join("b.txt"), "b").unwrap();
        let path = folder.to_string_lossy().to_string();

        assert_eq!(
            rename_local(&path, "a.txt", "b.txt"),
            Err(TAKEN.to_string())
        );
        assert_eq!(rename_local(&path, "a.txt", "c.txt"), Ok(()));
        assert!(folder.join("c.txt").exists() && !folder.join("a.txt").exists());
        assert!(rename_local(&path, "gone.txt", "d.txt").is_err());

        // The first item that fails stops the rest: b.txt stays.
        let result = delete_local(&path, &["gone.txt".to_string(), "b.txt".to_string()]);
        assert!(result.is_err());
        assert!(folder.join("b.txt").exists());
        assert_eq!(
            delete_local(&path, &["docs".to_string(), "b.txt".to_string()]),
            Ok(())
        );
        assert!(!folder.join("docs").exists() && !folder.join("b.txt").exists());
        assert!(folder.join("c.txt").exists());

        std::fs::remove_dir_all(&folder).unwrap();
    }
}
