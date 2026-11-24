//! The Files tab: two file lists side by side. Each side shows this PC, a host (SFTP) or an
//! available phone (ADB sync), or nothing yet: it then offers the list of places to choose
//! from. Files and folders are copied from one side to the other through the job list, by a
//! button, by dragging rows across or by dropping files from Explorer. Listing and copying is
//! done by the backend; this file keeps what each side shows, what is selected and which
//! copies are possible.

mod copy;
mod dragdrop;
mod entries;
mod folders;
mod local;
mod manage;
mod navigate;
mod open;
mod paths;
mod places;
mod present;
mod selection;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::{Model, VecModel};

use crate::{FileCrumb, FilePlace, FileRow, FileSide};

use entries::{row, view, Entry, Sort};

/// Where a side's files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Nothing chosen yet: the side offers the places to choose from.
    None,
    /// This computer.
    Local,
    /// A remote host, by id.
    Host(String),
    /// A phone, by route id.
    Device(String),
}

/// The kinds of place, which decide the path rules and which copies exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    None,
    Local,
    Host,
    Device,
}

impl Source {
    pub fn kind(&self) -> Kind {
        match self {
            Source::None => Kind::None,
            Source::Local => Kind::Local,
            Source::Host(_) => Kind::Host,
            Source::Device(_) => Kind::Device,
        }
    }

    /// The folder a side starts in.
    fn start_path(&self) -> String {
        match self {
            Source::None => String::new(),
            Source::Local => std::env::var("USERPROFILE").unwrap_or_default(),
            // SFTP resolves "." to the user's home folder.
            Source::Host(_) => ".".into(),
            Source::Device(_) => "/sdcard/".into(),
        }
    }
}

/// Whether the backend can copy from one kind of place to another. Two hosts work (also the
/// same host twice, for different folders); two phones, or this PC twice, do not.
pub fn pair_supported(from: Kind, to: Kind) -> bool {
    !matches!(
        (from, to),
        (Kind::None, _)
            | (_, Kind::None)
            | (Kind::Local, Kind::Local)
            | (Kind::Device, Kind::Device)
    )
}

/// How many folders a side remembers for Back.
const HISTORY: usize = 50;

struct Side {
    source: Source,
    /// The folder on screen, as the place reported it.
    path: String,
    /// Everything the folder holds, as it was listed.
    all: Vec<Entry>,
    /// The rows on screen: `all` through the filter, in the chosen order. Selections and row
    /// numbers refer to this list.
    entries: Vec<Entry>,
    rows: Rc<VecModel<FileRow>>,
    sort: Sort,
    filter: String,
    /// Bumped when Rust empties the filter, so the view empties its field.
    filter_rev: i32,
    /// Whether dotfiles (and hidden Windows items) are shown.
    show_hidden: bool,
    selected: BTreeSet<usize>,
    /// The row the keyboard is on, and where a Shift selection starts.
    cursor: Option<usize>,
    anchor: Option<usize>,
    loading: bool,
    /// A listing has been shown for this source.
    loaded: bool,
    error: String,
    /// Something the user should know that is not an error (why a drop did nothing).
    note: String,
    /// What went wrong in the action the listing on its way follows (some items could not be
    /// deleted): it becomes `error` when the listing arrives, which would otherwise clear it.
    outcome: String,
    /// Bumped by every request: an answer to an older one is dropped.
    request: u64,
    /// What the folder field should say, and a counter bumped each time that is set, so the
    /// view applies it even when the text is the same as before.
    input: String,
    input_rev: i32,
    /// Folders visited before this one, and the ones Back came from.
    back: Vec<String>,
    forward: Vec<String>,
    /// The listing under way was asked for by Back or Forward: it must not add to the history.
    travelling: bool,
    crumbs: Rc<VecModel<FileCrumb>>,
    /// The folder each crumb stands for.
    crumb_paths: Vec<String>,
    /// The list of places is shown instead of the files.
    picking: bool,
    query: String,
    /// The places on offer, in the order of `place_rows`.
    places: Vec<Source>,
    place_rows: Rc<VecModel<FilePlace>>,
    /// The key of the host must be accepted before its files can be listed.
    awaiting_trust: bool,
}

impl Side {
    fn new(source: Source) -> Self {
        Self {
            source,
            path: String::new(),
            all: Vec::new(),
            entries: Vec::new(),
            rows: Rc::new(VecModel::default()),
            sort: Sort::default(),
            filter: String::new(),
            filter_rev: 0,
            show_hidden: true,
            selected: BTreeSet::new(),
            cursor: None,
            anchor: None,
            loading: false,
            loaded: false,
            error: String::new(),
            note: String::new(),
            outcome: String::new(),
            request: 0,
            input: String::new(),
            input_rev: 0,
            back: Vec::new(),
            forward: Vec::new(),
            travelling: false,
            crumbs: Rc::new(VecModel::default()),
            crumb_paths: Vec::new(),
            picking: false,
            query: String::new(),
            places: Vec::new(),
            place_rows: Rc::new(VecModel::default()),
            awaiting_trust: false,
        }
    }

    /// Show another place: everything about the old one goes, the models the view holds stay.
    fn switch(&mut self, source: Source) {
        self.rows.set_vec(Vec::new());
        self.crumbs.set_vec(Vec::new());
        self.place_rows.set_vec(Vec::new());
        *self = Self {
            rows: self.rows.clone(),
            crumbs: self.crumbs.clone(),
            place_rows: self.place_rows.clone(),
            request: self.request,
            input_rev: self.input_rev.wrapping_add(1),
            filter_rev: self.filter_rev.wrapping_add(1),
            ..Self::new(source)
        };
    }

    fn set_input(&mut self, text: &str) {
        self.input = text.to_string();
        self.input_rev = self.input_rev.wrapping_add(1);
    }

    /// Rows of a host are two lines high: the permissions go under the name.
    fn tall(&self) -> bool {
        self.source.kind() == Kind::Host
    }

    fn set_selected(&mut self, selected: BTreeSet<usize>) {
        let tall = self.tall();
        for index in selected.symmetric_difference(&self.selected) {
            if let Some(entry) = self.entries.get(*index) {
                self.rows
                    .set_row_data(*index, row(entry, selected.contains(index), tall));
            }
        }
        self.selected = selected;
    }

    /// A listing arrived. A folder other than the one on screen empties the filter and, unless
    /// Back or Forward asked for it, goes on top of the history.
    fn show(&mut self, path: String, entries: Vec<Entry>) {
        let moved = self.loaded && self.path != path;
        if moved && !self.travelling {
            self.back.push(std::mem::take(&mut self.path));
            if self.back.len() > HISTORY {
                self.back.remove(0);
            }
            self.forward.clear();
        }
        if moved && !self.filter.is_empty() {
            self.filter.clear();
            self.filter_rev = self.filter_rev.wrapping_add(1);
        }
        self.travelling = false;
        self.path = path;
        self.all = entries;
        self.loaded = true;
        self.error = std::mem::take(&mut self.outcome);
        self.note.clear();
        self.arrange(false);
    }

    /// Put `all` through the filter and the sort order again. With `keep`, the rows that were
    /// selected stay selected wherever they end up.
    fn arrange(&mut self, keep: bool) {
        let names = |rows: &mut dyn Iterator<Item = usize>| -> Vec<String> {
            rows.filter_map(|row| self.entries.get(row))
                .map(|entry| entry.name.clone())
                .collect()
        };
        let selected = names(&mut self.selected.iter().copied());
        let cursor = names(&mut self.cursor.into_iter());
        let entries = view(&self.all, &self.filter, self.show_hidden, self.sort);
        let find = |name: &String| entries.iter().position(|entry| entry.name == *name);
        self.selected = if keep {
            selected.iter().filter_map(find).collect()
        } else {
            BTreeSet::new()
        };
        self.cursor = cursor.first().filter(|_| keep).and_then(find);
        self.anchor = self.cursor;
        let tall = self.tall();
        self.rows.set_vec(
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| row(entry, self.selected.contains(&index), tall))
                .collect::<Vec<_>>(),
        );
        self.entries = entries;
    }
}

/// What the Files tabs share: where the pointer last was, for files dropped from Explorer.
#[derive(Default)]
pub struct State {
    /// (pane id, side) the pointer was over last.
    hover: Cell<(i32, usize)>,
}

/// One Files tab.
pub struct FilesPane {
    pub id: i32,
    sides: [RefCell<Side>; 2],
    /// Opened as "SFTP": the tab keeps that name whatever its sides show.
    sftp: bool,
    /// What the view draws besides the rows: one row per side.
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
            sftp: false,
            info: Rc::new(VecModel::from(vec![
                FileSide::default(),
                FileSide::default(),
            ])),
        }
    }

    /// The SFTP layout: this PC on the left, and the right side waiting for a host to be
    /// chosen.
    pub fn sftp(id: i32) -> Self {
        Self {
            sftp: true,
            ..Self::new(id, Source::None)
        }
    }
}
