//! Minimal application log (NFR-07): one redacted line per event in
//! `<app log dir>/senvakara.log`, rotated at 5 MB. Terminal output and file
//! contents are never written here.

use senvakara_secrets::redact;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: u64 = 5 * 1024 * 1024;
static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();

pub fn init(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = LOG.set(Mutex::new(dir.join("senvakara.log")));
}

fn write(level: &str, msg: &str) {
    let line = format!(
        "{} {level} {}\n",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        redact(msg).replace('\n', " ")
    );
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    let Some(lock) = LOG.get() else { return };
    let path = lock.lock().unwrap();
    if std::fs::metadata(&*path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&*path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(msg: &str) {
    write("INFO", msg);
}

pub fn warn(msg: &str) {
    write("WARN", msg);
}
