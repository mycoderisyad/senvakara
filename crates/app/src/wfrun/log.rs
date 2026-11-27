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
    /// Secret values replaced before a line is shown or written (ADR-033).
    masks: Vec<String>,
    emit: F,
}

impl<F: FnMut(String)> NodeLog<F> {
    /// `emit` receives each line for the run tab, already prefixed. Each
    /// script of the node (check, apply, check, ...) appends to the same file,
    /// and the cap counts what earlier scripts wrote.
    pub fn new(dir: &Path, node_id: &str, prefix: &str, masks: Vec<String>, emit: F) -> Self {
        let file = std::fs::create_dir_all(dir).ok().and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{node_id}.log")))
                .ok()
        });
        let written = file
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map_or(0, |m| m.len() as usize);
        Self {
            prefix: prefix.to_string(),
            file,
            written,
            shown: 0,
            partial: [Vec::new(), Vec::new()],
            masks,
            emit,
        }
    }

    pub fn push(&mut self, stderr: bool, bytes: &[u8]) {
        let i = usize::from(stderr);
        self.partial[i].extend_from_slice(bytes);
        while let Some(end) = self.partial[i].iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial[i].drain(..=end).collect();
            self.line(stderr, &line);
        }
        // A line with no end for a long time is shown in pieces.
        if self.partial[i].len() > 4096 {
            let line = std::mem::take(&mut self.partial[i]);
            self.line(stderr, &line);
        }
    }

    /// One line (with or without its end): masked, written within the cap,
    /// then shown.
    fn line(&mut self, stderr: bool, bytes: &[u8]) {
        let raw = String::from_utf8_lossy(bytes);
        let text = super::secrets::apply_masks(&raw, &self.masks);
        if let Some(f) = self.file.as_mut() {
            let room = FILE_CAP.saturating_sub(self.written);
            let mut take = text.len().min(room);
            while !text.is_char_boundary(take) {
                take -= 1;
            }
            if take > 0 && f.write_all(&text.as_bytes()[..take]).is_ok() {
                self.written += take;
            }
        }
        let shown = text.trim_end().to_string();
        Self::show(&mut self.shown, &self.prefix, &mut self.emit, stderr, shown);
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
                self.line(stderr, &buf);
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
        let masks = vec!["hunter2-secret".to_string()];
        let mut log = NodeLog::new(&dir, "n1", "Install", masks, |l| lines.push(l));
        log.push(false, b"hel");
        log.push(true, b"warn\n");
        log.push(false, b"lo hunter2-");
        log.push(false, b"secret\nwor");
        log.finish();
        assert_eq!(
            lines,
            [
                "[Install] ! warn",
                "[Install] hello [secret]",
                "[Install] wor"
            ]
        );
        let text = std::fs::read_to_string(dir.join("n1.log")).unwrap();
        assert_eq!(text, "warn\nhello [secret]\nwor");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tab_and_the_file_are_both_bounded() {
        let dir = std::env::temp_dir().join(format!("svk-log-{}", uuid::Uuid::new_v4()));
        let mut count = 0;
        let mut log = NodeLog::new(&dir, "n1", "Flood", Vec::new(), |_| count += 1);
        let line = vec![b'x'; 1023];
        for _ in 0..3000 {
            log.push(false, &line);
            log.push(false, b"\n");
        }
        log.finish();
        assert_eq!(count, TAB_LINES);
        let size = std::fs::metadata(dir.join("n1.log")).unwrap().len() as usize;
        assert_eq!(size, FILE_CAP);
        // A later script of the same node adds nothing past the cap.
        let mut log = NodeLog::new(&dir, "n1", "Flood", Vec::new(), |_| {});
        log.push(false, b"more\n");
        log.finish();
        let size = std::fs::metadata(dir.join("n1.log")).unwrap().len() as usize;
        assert_eq!(size, FILE_CAP);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_script_of_a_node_lands_in_its_one_file() {
        let dir = std::env::temp_dir().join(format!("svk-log-{}", uuid::Uuid::new_v4()));
        for text in [
            &b"check: needs apply\n"[..],
            b"apply: done\n",
            b"check: ok\n",
        ] {
            let mut log = NodeLog::new(&dir, "n1", "Step", Vec::new(), |_| {});
            log.push(false, text);
            log.finish();
        }
        let text = std::fs::read_to_string(dir.join("n1.log")).unwrap();
        assert_eq!(text, "check: needs apply\napply: done\ncheck: ok\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
