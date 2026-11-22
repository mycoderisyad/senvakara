//! The entries of a listing: listing this PC, sorting, and the row the view draws for one.

use slint::SharedString;

use crate::FileRow;

use super::paths::tidy_local;

/// One row of a listing, whatever the place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// A drive of this PC ("C:\").
    pub is_drive: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: Option<i64>,
}

impl Entry {
    /// Whether opening it can show a folder. A link may point at one: trying is how to know.
    pub(super) fn opens(&self) -> bool {
        self.is_dir || self.is_drive || self.is_symlink
    }
}

/// Folders first, then by name without regard to case.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        (!a.is_dir, a.name.to_lowercase(), &a.name).cmp(&(
            !b.is_dir,
            b.name.to_lowercase(),
            &b.name,
        ))
    });
}

pub fn size_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let size = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if size < KB * KB {
        format!("{:.1} KB", size / KB)
    } else if size < KB * KB * KB {
        format!("{:.1} MB", size / (KB * KB))
    } else {
        format!("{:.2} GB", size / (KB * KB * KB))
    }
}

/// Local date and time, to the minute.
fn time_text(seconds: Option<i64>) -> String {
    seconds
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// The drives of this PC that answer.
fn list_drives() -> Vec<Entry> {
    ('A'..='Z')
        .map(|letter| format!("{letter}:\\"))
        .filter(|root| std::path::Path::new(root).exists())
        .map(|name| Entry {
            name,
            is_dir: true,
            is_symlink: false,
            is_drive: true,
            size: 0,
            modified: None,
        })
        .collect()
}

/// List a folder of this PC. Runs on a background thread. Returns the path as listed.
pub fn list_local(path: &str) -> Result<(String, Vec<Entry>), String> {
    let path = tidy_local(path);
    if path.is_empty() {
        return Ok((path, list_drives()));
    }
    let reader = std::fs::read_dir(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => "This folder does not exist.".to_string(),
        std::io::ErrorKind::PermissionDenied => {
            "Windows does not allow reading this folder.".to_string()
        }
        std::io::ErrorKind::NotADirectory => "This is a file, not a folder.".to_string(),
        _ => format!("The folder could not be read. ({error})"),
    })?;
    let mut entries = Vec::new();
    for item in reader.flatten() {
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        let is_symlink = item.file_type().is_ok_and(|kind| kind.is_symlink());
        // For a link, describe what it points at; a broken link is listed as a file.
        let meta = if is_symlink {
            std::fs::metadata(item.path()).or_else(|_| item.metadata())
        } else {
            item.metadata()
        };
        let Ok(meta) = meta else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs() as i64);
        entries.push(Entry {
            name,
            is_dir: meta.is_dir(),
            is_symlink,
            is_drive: false,
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    sort_entries(&mut entries);
    Ok((path, entries))
}

pub(super) fn row(entry: &Entry, selected: bool) -> FileRow {
    FileRow {
        name: entry.name.as_str().into(),
        kind: if entry.is_drive {
            "drive"
        } else if entry.is_dir {
            "folder"
        } else {
            "file"
        }
        .into(),
        link: entry.is_symlink,
        size: if entry.is_dir {
            SharedString::new()
        } else {
            size_text(entry.size).into()
        },
        modified: time_text(entry.modified).into(),
        selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size_text(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn lists_a_local_folder() {
        let dir = std::env::temp_dir().join(format!("senvakara-files-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("note.txt"), b"hello").unwrap();
        let (path, entries) = list_local(&format!("{}\\", dir.display())).unwrap();
        assert_eq!(path, dir.display().to_string());
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir && entries[0].name == "sub");
        assert_eq!((entries[1].name.as_str(), entries[1].size), ("note.txt", 5));
        assert!(entries[1].modified.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(list_local(&dir.display().to_string()).is_err());
        // The top of this PC is its drives.
        let (top, drives) = list_local("").unwrap();
        assert!(top.is_empty() && drives.iter().all(|drive| drive.is_drive));
    }
}
