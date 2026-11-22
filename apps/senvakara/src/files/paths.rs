//! Path arithmetic for this PC (Windows paths) and for hosts and phones (POSIX paths).

use super::Kind;

/// A Windows path as this pane writes it: backslashes, no trailing one except on a drive root.
/// The empty path is the list of drives.
pub(super) fn tidy_local(path: &str) -> String {
    let mut path = path.trim().replace('/', "\\");
    while path.len() > 3 && path.ends_with('\\') {
        path.pop();
    }
    // "C:" means the root of the drive here, not the current folder on it.
    if path.len() == 2 && path.ends_with(':') {
        path.push('\\');
    }
    path
}

/// A POSIX path without trailing slashes ("/" stays).
pub(super) fn tidy_posix(path: &str) -> String {
    let path = path.trim();
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/".into()
    } else {
        trimmed.to_string()
    }
}

/// `dir` and `name` as one path, in the rules of the place.
pub fn join(kind: Kind, dir: &str, name: &str) -> String {
    match kind {
        Kind::Local => {
            let dir = tidy_local(dir);
            if dir.is_empty() {
                // The entries of the drive list are roots already ("C:\").
                tidy_local(name)
            } else if dir.ends_with('\\') {
                format!("{dir}{name}")
            } else {
                format!("{dir}\\{name}")
            }
        }
        Kind::Host | Kind::Device | Kind::None => {
            let dir = tidy_posix(dir);
            if dir.ends_with('/') {
                format!("{dir}{name}")
            } else {
                format!("{dir}/{name}")
            }
        }
    }
}

/// The folder above `path`, or None at the top (the drive list, or "/").
pub fn parent(kind: Kind, path: &str) -> Option<String> {
    match kind {
        Kind::Local => {
            let path = tidy_local(path);
            if path.is_empty() {
                return None;
            }
            // A network share (\\server\share) sits directly under the drive list.
            if let Some(rest) = path.strip_prefix("\\\\") {
                let parts: Vec<&str> = rest.split('\\').filter(|part| !part.is_empty()).collect();
                if parts.len() <= 2 {
                    return Some(String::new());
                }
                return Some(format!("\\\\{}", parts[..parts.len() - 1].join("\\")));
            }
            match path.rfind('\\') {
                // "C:\" itself.
                Some(index) if index + 1 == path.len() => Some(String::new()),
                // "C:\Users": the drive root keeps its backslash.
                Some(2) => Some(path[..3].to_string()),
                Some(index) => Some(path[..index].to_string()),
                None => Some(String::new()),
            }
        }
        Kind::Host | Kind::Device | Kind::None => {
            let path = tidy_posix(path);
            if path == "/" || path.is_empty() {
                return None;
            }
            match path.rfind('/') {
                Some(0) => Some("/".into()),
                Some(index) => Some(path[..index].to_string()),
                None => None,
            }
        }
    }
}

/// Whether two paths of one place name the same folder.
pub(super) fn same_dir(kind: Kind, a: &str, b: &str) -> bool {
    match kind {
        Kind::Local => tidy_local(a).eq_ignore_ascii_case(&tidy_local(b)),
        Kind::Host | Kind::Device | Kind::None => tidy_posix(a) == tidy_posix(b),
    }
}

/// The folders from the top of a place down to `path`, as (label, folder): what the path bar
/// shows, each one a place to jump to.
pub(super) fn crumbs(kind: Kind, path: &str) -> Vec<(String, String)> {
    match kind {
        Kind::None => Vec::new(),
        Kind::Local => {
            let path = tidy_local(path);
            let mut crumbs = vec![("This PC".to_string(), String::new())];
            // A network share (\\server\share) is one step, like a drive.
            let (root, rest) = match path.strip_prefix("\\\\") {
                Some(share) => {
                    let mut parts = share.splitn(3, '\\');
                    let root = format!(
                        "\\\\{}\\{}",
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default()
                    );
                    (root, parts.next().unwrap_or_default().to_string())
                }
                None => match path.split_once('\\') {
                    Some((drive, rest)) => (format!("{drive}\\"), rest.to_string()),
                    None => (path.clone(), String::new()),
                },
            };
            if root.is_empty() {
                return crumbs;
            }
            crumbs.push((root.trim_end_matches('\\').to_string(), root.clone()));
            let mut folder = root;
            for part in rest.split('\\').filter(|part| !part.is_empty()) {
                folder = join(Kind::Local, &folder, part);
                crumbs.push((part.to_string(), folder.clone()));
            }
            crumbs
        }
        Kind::Host | Kind::Device => {
            let path = tidy_posix(path);
            let mut crumbs = vec![("/".to_string(), "/".to_string())];
            let mut folder = String::new();
            for part in path.split('/').filter(|part| !part.is_empty()) {
                folder = format!("{folder}/{part}");
                crumbs.push((part.to_string(), folder.clone()));
            }
            crumbs
        }
    }
}

/// Why `name` cannot be the name of a new folder, or "" when it can.
pub(super) fn folder_name_problem(kind: Kind, name: &str) -> &'static str {
    let name = name.trim();
    if name.is_empty() {
        return "Type a name for the folder.";
    }
    if name == "." || name == ".." {
        return "This name is taken by the folder itself.";
    }
    let forbidden: &[char] = match kind {
        Kind::Local => &['\\', '/', ':', '*', '?', '"', '<', '>', '|'],
        _ => &['/'],
    };
    if name.contains(forbidden) || name.contains('\0') {
        return match kind {
            Kind::Local => "A folder name cannot contain \\ / : * ? \" < > or |.",
            _ => "A folder name cannot contain a slash.",
        };
    }
    if kind == Kind::Local && name.ends_with('.') {
        return "A folder name cannot end with a dot on Windows.";
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths() {
        assert_eq!(join(Kind::Local, "", "C:\\"), "C:\\");
        assert_eq!(join(Kind::Local, "C:\\", "Users"), "C:\\Users");
        assert_eq!(join(Kind::Local, "C:\\Users", "me"), "C:\\Users\\me");
        assert_eq!(join(Kind::Local, "C:/Users/", "me"), "C:\\Users\\me");
        assert_eq!(parent(Kind::Local, ""), None);
        assert_eq!(parent(Kind::Local, "C:\\"), Some(String::new()));
        assert_eq!(parent(Kind::Local, "C:"), Some(String::new()));
        assert_eq!(parent(Kind::Local, "C:\\Users"), Some("C:\\".into()));
        assert_eq!(
            parent(Kind::Local, "C:\\Users\\me\\"),
            Some("C:\\Users".into())
        );
        assert_eq!(parent(Kind::Local, "\\\\nas\\share"), Some(String::new()));
        assert_eq!(
            parent(Kind::Local, "\\\\nas\\share\\films"),
            Some("\\\\nas\\share".into())
        );
        assert_eq!(tidy_local(" d: "), "d:\\");
    }

    #[test]
    fn posix_paths() {
        assert_eq!(join(Kind::Host, "/", "tmp"), "/tmp");
        assert_eq!(join(Kind::Host, "/home/me", "a.txt"), "/home/me/a.txt");
        assert_eq!(join(Kind::Host, "/home/me/", "a.txt"), "/home/me/a.txt");
        assert_eq!(parent(Kind::Host, "/"), None);
        assert_eq!(parent(Kind::Host, "/home"), Some("/".into()));
        assert_eq!(parent(Kind::Host, "/home/me"), Some("/home".into()));
        assert_eq!(parent(Kind::Host, "/home/me/"), Some("/home".into()));
    }

    #[test]
    fn android_paths() {
        assert_eq!(
            join(Kind::Device, "/sdcard/", "Download"),
            "/sdcard/Download"
        );
        assert_eq!(
            join(Kind::Device, "/sdcard/Download", "a b.txt"),
            "/sdcard/Download/a b.txt"
        );
        assert_eq!(parent(Kind::Device, "/sdcard/"), Some("/".into()));
        assert_eq!(
            parent(Kind::Device, "/storage/emulated/0"),
            Some("/storage/emulated".into())
        );
        assert_eq!(parent(Kind::Device, "/"), None);
        assert!(same_dir(
            Kind::Device,
            "/sdcard/Download/",
            "/sdcard/Download"
        ));
        assert!(same_dir(Kind::Local, "C:\\Temp\\", "c:/temp"));
        assert!(!same_dir(Kind::Host, "/tmp", "/Tmp"));
    }
}
