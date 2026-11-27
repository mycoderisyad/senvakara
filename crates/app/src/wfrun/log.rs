//! Output of one node: whole lines go to the run tab (a bounded number) and
//! everything up to a cap goes to `runs/<run id>/<node>.log` in the data
//! folder, so a long install log never fills memory or the tab.

use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Bytes of one node's output kept on disk.
pub(crate) const FILE_CAP: usize = 2 * 1024 * 1024;
/// Lines of one node's output shown in the run tab.
pub(crate) const TAB_LINES: usize = 2000;

pub(crate) struct NodeLog<F: FnMut(String)> {
    prefix: String,
    file: Option<File>,
    written: usize,
    shown: usize,
    /// A line still waiting for its end, per stream (stdout, stderr).
    partial: [Vec<u8>; 2],
    emit: F,
}

impl<F: FnMut(String)> NodeLog<F> {
    /// `emit` receives each line for the run tab, already prefixed.
    pub fn new(dir: &Path, node_id: &str, prefix: &str, emit: F) -> Self {
        let file = std::fs::create_dir_all(dir)
            .ok()
            .and_then(|()| File::create(dir.join(format!("{node_id}.log"))).ok());
        Self {
            prefix: prefix.to_string(),
            file,
            written: 0,
            shown: 0,
            partial: [Vec::new(), Vec::new()],
            emit,
        }
    }

    pub fn push(&mut self, stderr: bool, bytes: &[u8]) {
        if let Some(f) = self.file.as_mut() {
            let room = FILE_CAP.saturating_sub(self.written);
            let take = bytes.len().min(room);
            if take > 0 && f.write_all(&bytes[..take]).is_ok() {
                self.written += take;
            }
        }
        let buf = &mut self.partial[usize::from(stderr)];
        buf.extend_from_slice(bytes);
        while let Some(end) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line).trim_end().to_string();
            Self::show(&mut self.shown, &self.prefix, &mut self.emit, stderr, text);
        }
        // A line with no end for a long time is shown in pieces.
        if buf.len() > 4096 {
            let text = String::from_utf8_lossy(buf).to_string();
            buf.clear();
            Self::show(&mut self.shown, &self.prefix, &mut self.emit, stderr, text);
        }
    }

    fn show(shown: &mut usize, prefix: &str, emit: &mut F, stderr: bool, text: String) {
        *shown += 1;
        if *shown < TAB_LINES {
            let mark = if stderr { " !" } else { "" };
            emit(format!("[{prefix}]{mark} {text}"));
        } else if *shown == TAB_LINES {
            emit(format!("[{prefix}] (more output is in the run's log file)"));
        }
    }

    /// Shows what is left of unfinished lines.
    pub fn finish(mut self) {
        for stderr in [false, true] {
            let buf = std::mem::take(&mut self.partial[usize::from(stderr)]);
            if !buf.is_empty() {
                let text = String::from_utf8_lossy(&buf).trim_end().to_string();
                Self::show(&mut self.shown, &self.prefix, &mut self.emit, stderr, text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_split_across_chunks_and_streams_stay_apart() {
        let dir = std::env::temp_dir().join(format!("svk-log-{}", uuid::Uuid::new_v4()));
        let mut lines = Vec::new();
        let mut log = NodeLog::new(&dir, "n1", "Install", |l| lines.push(l));
        log.push(false, b"hel");
        log.push(true, b"warn\n");
        log.push(false, b"lo\nwor");
        log.finish();
        assert_eq!(
            lines,
            ["[Install] ! warn", "[Install] hello", "[Install] wor"]
        );
        let text = std::fs::read_to_string(dir.join("n1.log")).unwrap();
        assert_eq!(text, "helwarn\nlo\nwor");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tab_and_the_file_are_both_bounded() {
        let dir = std::env::temp_dir().join(format!("svk-log-{}", uuid::Uuid::new_v4()));
        let mut count = 0;
        let mut log = NodeLog::new(&dir, "n1", "Flood", |_| count += 1);
        let line = vec![b'x'; 1023];
        for _ in 0..3000 {
            log.push(false, &line);
            log.push(false, b"\n");
        }
        log.finish();
        assert_eq!(count, TAB_LINES);
        let size = std::fs::metadata(dir.join("n1.log")).unwrap().len() as usize;
        assert_eq!(size, FILE_CAP);
        let _ = std::fs::remove_dir_all(dir);
    }
}
