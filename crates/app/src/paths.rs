//! Where the app keeps its data. The defaults are the locations the v0.1.0
//! build used, so existing hosts, devices and workspaces carry over.

use std::path::{Path, PathBuf};

/// Directory name under the per-user data folders (the v0.1.0 identifier).
pub const APP_ID: &str = "dev.senvakara.desktop";
/// Overrides both the data and the log location (development, tests).
pub const DATA_DIR_ENV: &str = "SENVAKARA_DATA_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Folder of the database.
    pub data_dir: PathBuf,
    /// `<data_dir>/senvakara.db`.
    pub db_path: PathBuf,
    /// Folder of `senvakara.log`.
    pub log_dir: PathBuf,
    /// Whether `SENVAKARA_DATA_DIR` (or [`AppPaths::in_dir`]) chose the location.
    pub overridden: bool,
}

impl AppPaths {
    /// `SENVAKARA_DATA_DIR` if set, otherwise
    /// `%APPDATA%\dev.senvakara.desktop\senvakara.db` and
    /// `%LOCALAPPDATA%\dev.senvakara.desktop\logs\senvakara.log`.
    pub fn resolve() -> anyhow::Result<Self> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        if let Some(dir) = var(DATA_DIR_ENV) {
            return Ok(Self::in_dir(dir));
        }
        let roaming = var("APPDATA")
            .map(PathBuf::from)
            .or_else(|| var("XDG_DATA_HOME").map(PathBuf::from))
            .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .ok_or_else(|| anyhow::anyhow!("no per-user data folder (APPDATA is not set)"))?;
        let local = var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| roaming.clone());
        let data_dir = roaming.join(APP_ID);
        Ok(Self {
            db_path: data_dir.join("senvakara.db"),
            data_dir,
            log_dir: local.join(APP_ID).join("logs"),
            overridden: false,
        })
    }

    /// Everything under one folder: `<dir>/senvakara.db`, `<dir>/logs/`.
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        let data_dir = dir.as_ref().to_path_buf();
        Self {
            db_path: data_dir.join("senvakara.db"),
            log_dir: data_dir.join("logs"),
            data_dir,
            overridden: true,
        }
    }

    pub fn log_file(&self) -> PathBuf {
        self.log_dir.join("senvakara.log")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_puts_everything_in_one_folder() {
        let p = AppPaths::in_dir("X:/data");
        assert_eq!(p.db_path, PathBuf::from("X:/data").join("senvakara.db"));
        assert_eq!(
            p.log_file(),
            PathBuf::from("X:/data").join("logs").join("senvakara.log")
        );
        assert!(p.overridden);
    }

    #[cfg(windows)]
    #[test]
    fn default_matches_the_v010_locations() {
        if std::env::var_os(DATA_DIR_ENV).is_some() {
            return;
        }
        let p = AppPaths::resolve().unwrap();
        let roaming = PathBuf::from(std::env::var_os("APPDATA").unwrap());
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert_eq!(
            p.db_path,
            roaming.join("dev.senvakara.desktop").join("senvakara.db")
        );
        assert_eq!(
            p.log_file(),
            local
                .join("dev.senvakara.desktop")
                .join("logs")
                .join("senvakara.log")
        );
    }
}
