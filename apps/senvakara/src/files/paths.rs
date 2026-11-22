//! Path arithmetic for this PC (Windows paths) and for hosts and phones (POSIX paths).

use super::Kind;

// ------------------------------------------------------------------ paths

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
        Kind::Host | Kind::Device => {
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
        Kind::Host | Kind::Device => {
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
        Kind::Host | Kind::Device => tidy_posix(a) == tidy_posix(b),
    }
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
