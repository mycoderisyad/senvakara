//! The Files tab: two file lists side by side. Each side shows this PC, a connected host (SFTP)
//! or an available phone (ADB sync), and files are copied from one side to the other through
//! the job list. Listing and copying is done by the backend; this file keeps what each side
//! shows, what is selected and which copies are possible.

mod copy;
mod entries;
mod navigate;
mod open;
mod paths;
mod present;
mod selection;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::{Model, SharedString, VecModel};

use crate::{FileRow, FileSide};

use entries::{row, Entry};

/// Where a side's files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// This computer.
    Local,
    /// A remote host, by id.
    Host(String),
    /// A phone, by route id.
    Device(String),
}

/// The three kinds of place, which decide the path rules and which copies exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Local,
    Host,
    Device,
}

impl Source {
    pub fn kind(&self) -> Kind {
        match self {
            Source::Local => Kind::Local,
            Source::Host(_) => Kind::Host,
            Source::Device(_) => Kind::Device,
        }
    }

    /// The folder a side starts in.
    fn start_path(&self) -> String {
        match self {
            Source::Local => std::env::var("USERPROFILE").unwrap_or_default(),
            // SFTP resolves "." to the user's home folder.
            Source::Host(_) => ".".into(),
            Source::Device(_) => "/sdcard/".into(),
        }
    }
}

/// Whether the backend can copy a file from one kind of place to another.
pub fn pair_supported(from: Kind, to: Kind) -> bool {
    from != to
}

// ------------------------------------------------------------------- pane

struct Side {
    source: Source,
    /// The folder on screen, as the place reported it.
    path: String,
    entries: Vec<Entry>,
    rows: Rc<VecModel<FileRow>>,
    selected: BTreeSet<usize>,
    /// The row the keyboard is on, and where a Shift selection starts.
    cursor: Option<usize>,
    anchor: Option<usize>,
    loading: bool,
    /// A listing has been shown for this source.
    loaded: bool,
    error: String,
    /// Bumped by every request: an answer to an older one is dropped.
    request: u64,
    /// What the folder field should say, and a counter bumped each time that is set, so the
    /// view applies it even when the text is the same as before.
    input: String,
    input_rev: i32,
}

impl Side {
    fn new(source: Source) -> Self {
        Self {
            source,
            path: String::new(),
            entries: Vec::new(),
            rows: Rc::new(VecModel::default()),
            selected: BTreeSet::new(),
            cursor: None,
            anchor: None,
            loading: false,
            loaded: false,
            error: String::new(),
            request: 0,
            input: String::new(),
            input_rev: 0,
        }
    }

    fn set_input(&mut self, text: &str) {
        self.input = text.to_string();
        self.input_rev = self.input_rev.wrapping_add(1);
    }

    fn set_selected(&mut self, selected: BTreeSet<usize>) {
        for index in selected.symmetric_difference(&self.selected) {
            if let Some(entry) = self.entries.get(*index) {
                self.rows
                    .set_row_data(*index, row(entry, selected.contains(index)));
            }
        }
        self.selected = selected;
    }

    fn show(&mut self, path: String, entries: Vec<Entry>) {
        self.rows
            .set_vec(entries.iter().map(|e| row(e, false)).collect::<Vec<_>>());
        self.path = path;
        self.entries = entries;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.loaded = true;
        self.error.clear();
    }
}

/// One Files tab.
pub struct FilesPane {
    pub id: i32,
    sides: [RefCell<Side>; 2],
    /// The places of the source picker, in its order.
    sources: RefCell<Vec<Source>>,
    /// What the view draws besides the rows: the picker's labels and one row per side.
    labels: Rc<VecModel<SharedString>>,
    info: Rc<VecModel<FileSide>>,
}

impl FilesPane {
    /// A pane with this PC on the left and `source` on the right. Nothing is listed until
    /// [`App::files_start`] runs, once the pane is in a tab or a workspace panel.
    pub fn new(id: i32, source: Source) -> Self {
        Self {
            id,
            sides: [
                RefCell::new(Side::new(Source::Local)),
                RefCell::new(Side::new(source)),
            ],
            sources: RefCell::new(Vec::new()),
            labels: Rc::new(VecModel::default()),
            info: Rc::new(VecModel::from(vec![
                FileSide::default(),
                FileSide::default(),
            ])),
        }
    }
}
