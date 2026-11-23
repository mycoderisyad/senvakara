//! One logcat pane: the log stream of a phone, kept as the last [`CAPACITY`] lines.
//!
//! The backend thread only appends bytes to a shared buffer and queues one refresh; splitting
//! into lines, the ring buffer, the filter and the Slint model all live on the UI thread, so a
//! flood of output costs one model update per refresh, not one per line.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use senvakara_app::{LogcatEvent, Sink};
use slint::{Model, ModelNotify, ModelTracker};

use crate::LogLine;

/// How many lines a pane keeps.
pub const CAPACITY: usize = 5000;

/// What one batch of output changed in the list of shown (filtered) lines.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Delta {
    /// Shown lines that fell out at the front.
    pub removed_front: usize,
    /// Shown lines to add at the end.
    pub appended: Vec<String>,
}

/// The last `capacity` lines of a byte stream, and which of them match the filter.
pub struct LogBuffer {
    lines: VecDeque<String>,
    /// Bytes after the last line break: an unfinished line, possibly an unfinished character.
    partial: Vec<u8>,
    capacity: usize,
    /// Lower-cased; empty shows everything.
    filter: String,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            partial: Vec::new(),
            capacity: capacity.max(1),
            filter: String::new(),
        }
    }

    pub fn total(&self) -> usize {
        self.lines.len()
    }

    fn matches(&self, line: &str) -> bool {
        self.filter.is_empty() || line.to_lowercase().contains(&self.filter)
    }

    /// Add output as it arrived, in chunks that may end in the middle of a line.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Delta {
        let mut delta = Delta::default();
        self.partial.extend_from_slice(bytes);
        let Some(last_break) = self.partial.iter().rposition(|byte| *byte == b'\n') else {
            return delta;
        };
        let rest = self.partial.split_off(last_break + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        let mut incoming: Vec<&[u8]> = complete[..last_break]
            .split(|byte| *byte == b'\n')
            .collect();
        // More than fits: only the newest lines can stay, and everything older goes.
        if incoming.len() > self.capacity {
            incoming.drain(..incoming.len() - self.capacity);
        }
        let overflow = (self.lines.len() + incoming.len()).saturating_sub(self.capacity);
        let dropped: Vec<String> = self.lines.drain(..overflow.min(self.lines.len())).collect();
        delta.removed_front = dropped.iter().filter(|line| self.matches(line)).count();
        for raw in incoming {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            let line = String::from_utf8_lossy(raw).into_owned();
            if self.matches(&line) {
                delta.appended.push(line.clone());
            }
            self.lines.push_back(line);
        }
        delta
    }

    /// Change the filter (case-insensitive substring). Returns every line that matches now.
    pub fn set_filter(&mut self, filter: &str) -> Vec<String> {
        self.filter = filter.trim().to_lowercase();
        self.shown()
    }

    pub fn shown(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|line| self.matches(line))
            .cloned()
            .collect()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }
}

/// The priority of a `logcat -v threadtime` line ("11-22 10:00:00.000  123  456 E tag: text"):
/// "error" for E and F, "warn" for W, "" for everything else and for lines in another format.
pub fn level_of(line: &str) -> &'static str {
    match line.split_ascii_whitespace().nth(4) {
        Some("E") | Some("F") => "error",
        Some("W") => "warn",
        _ => "",
    }
}

/// A line as the list draws it. Tabs (stack traces start with one) have no glyph in the log
/// face: they become spaces.
fn row(text: String) -> LogLine {
    LogLine {
        level: level_of(&text).into(),
        text: if text.contains('\t') {
            text.replace('\t', "    ").into()
        } else {
            text.into()
        },
    }
}

/// The shown lines as a Slint model that can drop rows at the front without rebuilding.
#[derive(Default)]
pub struct LogModel {
    rows: RefCell<VecDeque<LogLine>>,
    notify: ModelNotify,
}

impl LogModel {
    fn apply(&self, delta: Delta) {
        if delta.removed_front > 0 {
            let mut rows = self.rows.borrow_mut();
            let count = delta.removed_front.min(rows.len());
            rows.drain(..count);
            drop(rows);
            self.notify.row_removed(0, count);
        }
        if !delta.appended.is_empty() {
            let mut rows = self.rows.borrow_mut();
            let (at, count) = (rows.len(), delta.appended.len());
            rows.extend(delta.appended.into_iter().map(row));
            drop(rows);
            self.notify.row_added(at, count);
        }
    }

    /// Length of the longest row in characters, for sideways scrolling.
    pub fn max_chars(&self) -> usize {
        self.rows
            .borrow()
            .iter()
            .map(|line| line.text.chars().count())
            .max()
            .unwrap_or(0)
    }

    fn replace(&self, lines: Vec<String>) {
        *self.rows.borrow_mut() = lines.into_iter().map(row).collect();
        self.notify.reset();
    }
}

impl Model for LogModel {
    type Data = LogLine;

    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<LogLine> {
        self.rows.borrow().get(row).cloned()
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

enum Signal {
    Started,
    Failed(String),
    Ended,
}

struct Shared {
    pending: Mutex<Vec<u8>>,
    signals: Mutex<Vec<Signal>>,
    dirty: AtomicBool,
    /// The backend's stream id once it has started, -1 otherwise.
    remote: AtomicI64,
    /// Bumped on every start and stop: output of an older stream is refused.
    generation: AtomicU32,
    closed: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn mark(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }

    fn signal(&self, signal: Signal) {
        self.signals.lock().unwrap().push(signal);
        self.mark();
    }
}

pub struct LogcatPane {
    pub id: i32,
    pub title: String,
    pub route: String,
    pub hue: i32,
    backend: Arc<senvakara_app::App>,
    route_id: String,
    shared: Arc<Shared>,
    buffer: RefCell<LogBuffer>,
    pub model: Rc<LogModel>,
    pub filter: RefCell<String>,
    pub follow: Cell<bool>,
    pub starting: Cell<bool>,
    pub streaming: Cell<bool>,
    pub error: RefCell<String>,
    /// Bumped whenever the view should scroll to the newest line.
    pub revision: Cell<i32>,
}

impl LogcatPane {
    /// Open the pane and start streaming. `notify` must only queue a refresh on the UI thread.
    pub fn start(
        id: i32,
        backend: Arc<senvakara_app::App>,
        route_id: String,
        title: String,
        route: String,
        hue: i32,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let pane = Self {
            id,
            title,
            route,
            hue,
            backend,
            route_id,
            shared: Arc::new(Shared {
                pending: Mutex::new(Vec::new()),
                signals: Mutex::new(Vec::new()),
                dirty: AtomicBool::new(false),
                remote: AtomicI64::new(-1),
                generation: AtomicU32::new(0),
                closed: AtomicBool::new(false),
                notify: Box::new(notify),
            }),
            buffer: RefCell::new(LogBuffer::new(CAPACITY)),
            model: Rc::new(LogModel::default()),
            filter: RefCell::new(String::new()),
            follow: Cell::new(true),
            starting: Cell::new(false),
            streaming: Cell::new(false),
            error: RefCell::new(String::new()),
            revision: Cell::new(0),
        };
        pane.resume();
        pane
    }

    /// Whether a stream is running or about to.
    pub fn active(&self) -> bool {
        self.starting.get() || self.streaming.get()
    }

    /// Start streaming again. The backend replays the last 500 lines first, so the lines of
    /// the earlier stream are dropped.
    pub fn resume(&self) {
        if self.active() {
            return;
        }
        self.starting.set(true);
        self.error.borrow_mut().clear();
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        // The backend replays the last lines first: keeping the old ones would show them twice.
        self.shared.pending.lock().unwrap().clear();
        self.clear();
        let current = {
            let shared = self.shared.clone();
            move || {
                !shared.closed.load(Ordering::Acquire)
                    && shared.generation.load(Ordering::Acquire) == generation
            }
        };
        let sink: Sink<LogcatEvent> = {
            let (shared, current) = (self.shared.clone(), current.clone());
            Arc::new(move |event| {
                if !current() {
                    return false;
                }
                match event {
                    LogcatEvent::Data(bytes) => {
                        shared.pending.lock().unwrap().extend_from_slice(&bytes);
                        shared.mark();
                    }
                    LogcatEvent::Ended => shared.signal(Signal::Ended),
                }
                true
            })
        };
        let (backend, shared, route_id) = (
            self.backend.clone(),
            self.shared.clone(),
            self.route_id.clone(),
        );
        self.backend.rt.spawn(async move {
            match backend.logcat_start(route_id, sink).await {
                // Stopped or closed while it was starting: end the stream nobody reads.
                Ok(remote) if !current() => backend.logcat_stop(remote),
                Ok(remote) => {
                    shared.remote.store(remote as i64, Ordering::Release);
                    shared.signal(Signal::Started);
                }
                Err(error) if current() => {
                    shared.signal(Signal::Failed(crate::inventory::friendly(&error)));
                }
                Err(_) => {}
            }
        });
    }

    /// Stop streaming. The lines stay.
    pub fn stop(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        let remote = self.shared.remote.swap(-1, Ordering::AcqRel);
        if remote >= 0 {
            self.backend.logcat_stop(remote as u32);
        }
        self.starting.set(false);
        self.streaming.set(false);
    }

    /// Take in what arrived since the last call. Returns true when the shown lines changed.
    /// UI thread only.
    pub fn take(&self) -> bool {
        self.shared.dirty.store(false, Ordering::Release);
        let bytes = std::mem::take(&mut *self.shared.pending.lock().unwrap());
        let signals = std::mem::take(&mut *self.shared.signals.lock().unwrap());
        let mut changed = false;
        if !bytes.is_empty() {
            let delta = self.buffer.borrow_mut().push_bytes(&bytes);
            changed = delta != Delta::default();
            self.model.apply(delta);
        }
        for signal in signals {
            self.starting.set(false);
            match signal {
                Signal::Started => self.streaming.set(true),
                Signal::Failed(reason) => {
                    self.streaming.set(false);
                    *self.error.borrow_mut() = reason;
                }
                Signal::Ended => {
                    self.streaming.set(false);
                    self.shared.remote.store(-1, Ordering::Release);
                    *self.error.borrow_mut() =
                        "The log stream ended. The phone was probably disconnected.".into();
                }
            }
        }
        changed
    }

    pub fn set_filter(&self, filter: &str) {
        *self.filter.borrow_mut() = filter.to_string();
        let shown = self.buffer.borrow_mut().set_filter(filter);
        self.model.replace(shown);
    }

    pub fn clear(&self) {
        self.buffer.borrow_mut().clear();
        self.model.replace(Vec::new());
    }

    /// (tone, sentence) for the status mark.
    pub fn status(&self) -> (&'static str, String) {
        let (shown, total) = (self.model.row_count(), self.buffer.borrow().total());
        let (tone, word) = if self.starting.get() {
            ("busy", "Starting")
        } else if self.streaming.get() {
            ("live", "Streaming")
        } else {
            ("idle", "Stopped")
        };
        (tone, format!("{word}, {shown} of {total} lines"))
    }
}

impl LogcatPane {
    /// What the list says while it has no lines to show.
    pub fn empty_text(&self) -> &'static str {
        if self.buffer.borrow().total() > 0 {
            "No line matches the filter."
        } else if self.starting.get() {
            "Starting the log stream."
        } else if self.streaming.get() {
            "Waiting for log lines."
        } else {
            "No lines. Start the stream to read the phone's log."
        }
    }
}

impl Drop for LogcatPane {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(buffer: &LogBuffer) -> Vec<String> {
        buffer.lines.iter().cloned().collect()
    }

    #[test]
    fn splits_lines_across_chunks() {
        let mut buffer = LogBuffer::new(10);
        assert_eq!(buffer.push_bytes(b"first li"), Delta::default());
        let delta = buffer.push_bytes(b"ne\r\nsecond\nthi");
        assert_eq!(delta.appended, ["first line", "second"]);
        assert_eq!(buffer.total(), 2);
        assert_eq!(buffer.push_bytes(b"rd\n").appended, ["third"]);
    }

    #[test]
    fn keeps_a_character_split_between_chunks() {
        let mut buffer = LogBuffer::new(10);
        let text = "caf\u{e9} ok\n".as_bytes();
        buffer.push_bytes(&text[..4]);
        let delta = buffer.push_bytes(&text[4..]);
        assert_eq!(delta.appended, ["caf\u{e9} ok"]);
    }

    #[test]
    fn keeps_only_the_newest_lines() {
        let mut buffer = LogBuffer::new(3);
        buffer.push_bytes(b"1\n2\n");
        let delta = buffer.push_bytes(b"3\n4\n5\n");
        assert_eq!(delta.removed_front, 2);
        assert_eq!(delta.appended, ["3", "4", "5"]);
        assert_eq!(lines(&buffer), ["3", "4", "5"]);
    }

    #[test]
    fn one_chunk_larger_than_the_buffer() {
        let mut buffer = LogBuffer::new(3);
        buffer.push_bytes(b"old\n");
        let delta = buffer.push_bytes(b"1\n2\n3\n4\n5\n");
        assert_eq!(delta.removed_front, 1);
        assert_eq!(delta.appended, ["3", "4", "5"]);
        assert_eq!(buffer.total(), 3);
    }

    #[test]
    fn filter_ignores_case_and_tracks_dropped_lines() {
        let mut buffer = LogBuffer::new(3);
        buffer.push_bytes(b"Wifi up\nother\n");
        assert_eq!(buffer.set_filter("  WIFI "), ["Wifi up"]);
        // Two lines fall out: only one of them was shown.
        let delta = buffer.push_bytes(b"wifi down\nnoise\nmore\n");
        assert_eq!(delta.removed_front, 1);
        assert_eq!(delta.appended, ["wifi down"]);
        assert_eq!(buffer.set_filter(""), ["wifi down", "noise", "more"]);
    }

    #[test]
    fn clear_drops_the_unfinished_line_too() {
        let mut buffer = LogBuffer::new(3);
        buffer.push_bytes(b"done\nhalf");
        buffer.clear();
        assert_eq!(buffer.push_bytes(b" tail\n").appended, [" tail"]);
        assert_eq!(buffer.total(), 1);
    }

    #[test]
    fn a_replay_after_a_restart_does_not_repeat_lines() {
        let mut buffer = LogBuffer::new(10);
        let model = LogModel::default();
        model.apply(buffer.push_bytes(
            b"one
two
",
        ));
        // What `LogcatPane::clear` does when a stopped stream starts again.
        buffer.clear();
        model.replace(Vec::new());
        // The backend replays what the phone still has.
        model.apply(buffer.push_bytes(
            b"one
two
three
",
        ));
        assert_eq!(buffer.total(), 3);
        let shown: Vec<String> = model.iter().map(|line| line.text.to_string()).collect();
        assert_eq!(shown, ["one", "two", "three"]);
    }

    #[test]
    fn reads_the_priority_of_threadtime_lines() {
        assert_eq!(
            level_of("11-22 10:00:00.000  1234  5678 E AndroidRuntime: boom"),
            "error"
        );
        assert_eq!(
            level_of("11-22 10:00:00.000  1234  5678 W System: slow"),
            "warn"
        );
        assert_eq!(
            level_of("11-22 10:00:00.000  1234  5678 I System: fine"),
            ""
        );
        assert_eq!(level_of("--------- beginning of main"), "");
        assert_eq!(level_of(""), "");
    }

    /// A flood: far more output than the buffer keeps, in the 32 KB chunks the backend sends.
    #[test]
    fn a_flood_keeps_the_newest_lines_and_the_model_in_step() {
        let mut text = String::new();
        for n in 0..200_000 {
            text.push_str(&format!(
                "11-22 10:00:00.000  1234  5678 I Flood   : line {n}\n"
            ));
        }
        let mut buffer = LogBuffer::new(CAPACITY);
        buffer.set_filter("line 1999");
        let model = LogModel::default();
        let started = std::time::Instant::now();
        for chunk in text.as_bytes().chunks(32 * 1024) {
            model.apply(buffer.push_bytes(chunk));
        }
        println!("200000 lines in {:?}", started.elapsed());
        assert_eq!(buffer.total(), CAPACITY);
        assert!(lines(&buffer)[0].ends_with("line 195000"));
        // 199900..=199999, and 1999 itself is long gone.
        assert_eq!(model.row_count(), 100);
        assert!(model.row_data(0).unwrap().text.ends_with("line 199900"));
        assert_eq!(buffer.shown().len(), model.row_count());
    }

    #[test]
    fn model_follows_the_deltas() {
        let model = LogModel::default();
        model.apply(Delta {
            removed_front: 0,
            appended: vec!["a".into(), "b".into(), "c".into()],
        });
        model.apply(Delta {
            removed_front: 2,
            appended: vec!["d".into()],
        });
        let shown: Vec<String> = model.iter().map(|line| line.text.to_string()).collect();
        assert_eq!(shown, ["c", "d"]);
        model.apply(Delta {
            removed_front: 0,
            appended: vec!["\tat com.example.Main".into()],
        });
        assert_eq!(model.row_data(2).unwrap().text, "    at com.example.Main");
        assert_eq!(model.max_chars(), 23);
        model.replace(Vec::new());
        assert_eq!(model.row_count(), 0);
    }
}
