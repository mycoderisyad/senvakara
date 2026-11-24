//! The entries of a listing: listing this PC, filtering, sorting, and the row the view draws
//! for one.

use std::cmp::Ordering;

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
    /// A dotfile or an item with the Windows Hidden attribute.
    pub is_hidden: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: Option<i64>,
    /// Type and permissions as a host reports them ("drwxr-xr-x"); empty elsewhere.
    pub permissions: String,
}

impl Entry {
    /// Whether opening it can show a folder. A link may point at one: trying is how to know.
    pub(super) fn opens(&self) -> bool {
        self.is_dir || self.is_drive || self.is_symlink
    }

    /// A folder or a drive: listed before the files, and something files can be dropped into.
    pub(super) fn holds_files(&self) -> bool {
        self.is_dir || self.is_drive
    }

    /// The Kind column: "folder", "drive", "link", or the file's extension ("pdf"), "file"
    /// when it has none.
    pub(super) fn kind_text(&self) -> String {
        if self.is_drive {
            return "drive".into();
        }
        if self.is_dir {
            return "folder".into();
        }
        if self.is_symlink {
            return "link".into();
        }
        match self.name.rsplit_once('.') {
            // ".bashrc" is a name, not an extension.
            Some((stem, extension))
                if !stem.is_empty() && !extension.is_empty() && extension.len() <= 8 =>
            {
                extension.to_lowercase()
            }
            _ => "file".into(),
        }
    }
}

/// The column a list is ordered by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Column {
    #[default]
    Name,
    Modified,
    Size,
    Kind,
}

impl Column {
    /// The name the view uses for the column.
    pub(super) fn id(self) -> &'static str {
        match self {
            Column::Name => "name",
            Column::Modified => "modified",
            Column::Size => "size",
            Column::Kind => "kind",
        }
    }

    pub(super) fn from_id(id: &str) -> Option<Self> {
        [Column::Name, Column::Modified, Column::Size, Column::Kind]
            .into_iter()
            .find(|column| column.id() == id)
    }
}

/// How a list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Sort {
    /// The order after a click on the header of `column`: a new column starts ascending, the
    /// same one turns around.
    pub(super) fn clicked(self, column: Column) -> Self {
        Self {
            column,
            descending: self.column == column && !self.descending,
        }
    }
}

/// Folders first, whatever the direction; then by the chosen column, and by name without
/// regard to case where that leaves two rows equal.
pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    let by_name = |a: &Entry, b: &Entry| {
        (a.name.to_lowercase(), &a.name).cmp(&(b.name.to_lowercase(), &b.name))
    };
    entries.sort_by(|a, b| {
        let folders = b.holds_files().cmp(&a.holds_files());
        if folders != Ordering::Equal {
            return folders;
        }
        let order = match sort.column {
            Column::Name => Ordering::Equal,
            Column::Modified => a.modified.cmp(&b.modified),
            Column::Size => a.size.cmp(&b.size),
            Column::Kind => a.kind_text().cmp(&b.kind_text()),
        }
        .then_with(|| by_name(a, b));
        if sort.descending {
            order.reverse()
        } else {
            order
        }
    });
}

/// The rows of a list: the entries whose name contains `filter` (case does not matter), with
/// hidden items omitted when `show_hidden` is false, in the order of `sort`.
pub fn view(all: &[Entry], filter: &str, show_hidden: bool, sort: Sort) -> Vec<Entry> {
    let needle = filter.trim().to_lowercase();
    let mut entries: Vec<Entry> = all
        .iter()
        .filter(|entry| show_hidden || !entry.is_hidden)
        .filter(|entry| needle.is_empty() || entry.name.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    sort_entries(&mut entries, sort);
    entries
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
            is_hidden: false,
            size: 0,
            modified: None,
            permissions: String::new(),
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
        let is_hidden = is_local_hidden(&name, &meta);
        entries.push(Entry {
            name,
            is_dir: meta.is_dir(),
            is_symlink,
            is_drive: false,
            is_hidden,
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
            permissions: String::new(),
        });
    }
    sort_entries(&mut entries, Sort::default());
    Ok((path, entries))
}

fn is_local_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_HIDDEN = 0x2.
        (meta.file_attributes() & 0x2) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// The row of an entry. `tall` rows have a second line under the name, for the permissions.
pub(super) fn row(entry: &Entry, selected: bool, tall: bool) -> FileRow {
    FileRow {
        name: entry.name.as_str().into(),
        detail: if tall {
            entry.permissions.as_str().into()
        } else {
            SharedString::new()
        },
        kind_text: entry.kind_text().into(),
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
