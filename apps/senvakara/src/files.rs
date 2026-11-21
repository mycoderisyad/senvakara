//! The Files tab: two file lists side by side. Each side shows this PC, a connected host (SFTP)
//! or an available phone (ADB sync), and files are copied from one side to the other through
//! the job list. Listing and copying is done by the backend; this file keeps what each side
//! shows, what is selected and which copies are possible.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::platform::Key;
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::device::target_facts;
use crate::inventory::{fact, friendly, transport_label};
use crate::jobs::JobSpec;
use crate::{App, FileRow, FileSide, Files, Pane, Pending, Place, Question, TabInfo};

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

// ------------------------------------------------------------------ paths

/// A Windows path as this pane writes it: backslashes, no trailing one except on a drive root.
/// The empty path is the list of drives.
fn tidy_local(path: &str) -> String {
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
fn tidy_posix(path: &str) -> String {
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
fn same_dir(kind: Kind, a: &str, b: &str) -> bool {
    match kind {
        Kind::Local => tidy_local(a).eq_ignore_ascii_case(&tidy_local(b)),
        Kind::Host | Kind::Device => tidy_posix(a) == tidy_posix(b),
    }
}

// ---------------------------------------------------------------- entries

/// One row of a listing, whatever the place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// A drive of this PC ("C:\").
    pub is_drive: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: Option<i64>,
}

impl Entry {
    /// Whether opening it can show a folder. A link may point at one: trying is how to know.
    fn opens(&self) -> bool {
        self.is_dir || self.is_drive || self.is_symlink
    }
}

/// Folders first, then by name without regard to case.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        (!a.is_dir, a.name.to_lowercase(), &a.name).cmp(&(
            !b.is_dir,
            b.name.to_lowercase(),
            &b.name,
        ))
    });
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
            size: 0,
            modified: None,
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
        entries.push(Entry {
            name,
            is_dir: meta.is_dir(),
            is_symlink,
            is_drive: false,
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified,
        });
    }
    sort_entries(&mut entries);
    Ok((path, entries))
}

fn row(entry: &Entry, selected: bool) -> FileRow {
    FileRow {
        name: entry.name.as_str().into(),
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

/// Why the selection of one side cannot be copied to the other, or "" when it can.
fn copy_problem(
    from: Kind,
    to: Kind,
    selected: &[&Entry],
    to_loaded: bool,
    to_path: &str,
    phone_ready: bool,
) -> &'static str {
    if !pair_supported(from, to) {
        return match from {
            Kind::Local => "Not supported yet: both sides show this PC.",
            Kind::Host => "Not supported yet: copying from one host to another.",
            Kind::Device => "Not supported yet: copying from one phone to another.",
        };
    }
    if !phone_ready {
        return "The phone must be connected and authorized first.";
    }
    if !to_loaded {
        return "The other side has no folder open yet.";
    }
    if to == Kind::Local && to_path.is_empty() {
        return "Open a folder on the other side first. Files cannot be copied to the list of drives.";
    }
    if selected.is_empty() {
        return "Select one or more files first.";
    }
    if selected.iter().any(|entry| entry.is_dir || entry.is_drive) {
        return "Folders cannot be copied yet. Select files only.";
    }
    ""
}

impl App {
    pub fn init_files(self: &Rc<Self>, ui: &crate::MainWindow) {
        let files = ui.global::<Files>();
        files.on_pick_source({
            let app = self.clone();
            move |id, side, index| app.files_pick_source(id, side as usize, index)
        });
        files.on_go({
            let app = self.clone();
            move |id, side, path| app.files_go(id, side as usize, path.to_string())
        });
        files.on_up({
            let app = self.clone();
            move |id, side| app.files_up(id, side as usize)
        });
        files.on_refresh({
            let app = self.clone();
            move |id, side| app.files_refresh(id, side as usize)
        });
        files.on_open({
            let app = self.clone();
            move |id, side, row| app.files_open(id, side as usize, row as usize)
        });
        files.on_select({
            let app = self.clone();
            move |id, side, row, control, shift| {
                app.files_select(id, side as usize, row as usize, control, shift)
            }
        });
        files.on_key({
            let app = self.clone();
            move |id, side, text, control, shift| {
                app.files_key(id, side as usize, &text, control, shift)
            }
        });
        files.on_copy({
            let app = self.clone();
            move |id, side| app.files_copy(id, side as usize)
        });
    }

    /// Open a Files tab with this PC on the left and `source` on the right.
    pub fn open_files(self: &Rc<Self>, source: Source) {
        let id = self.take_id();
        let title = tab_title(
            Kind::Local,
            "This PC",
            source.kind(),
            &self.place_name(&source),
        );
        let hue = match &source {
            Source::Local => 0,
            Source::Host(host) => self.inv.hue(host),
            Source::Device(route) => self
                .inv
                .device(route)
                .map(|device| self.inv.hue(&device.record.host_id))
                .unwrap_or(0),
        };
        self.sessions
            .borrow_mut()
            .push(Pane::Files(FilesPane::new(id, source)));
        self.tabs.push(TabInfo {
            id,
            kind: "files".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
        self.files_start(id);
    }

    /// List the starting folder of both sides of a pane that was just put in place.
    pub fn files_start(self: &Rc<Self>, id: i32) {
        for side in 0..2 {
            let source = self.with_files(id, |pane| pane.sides[side].borrow().source.clone());
            let Some(source) = source else { return };
            let path = source.start_path();
            self.files_load(id, side, source, path);
        }
    }

    /// What a place is called in the picker, in labels and in confirmations.
    fn place_name(&self, source: &Source) -> String {
        match source {
            Source::Local => "This PC".into(),
            Source::Host(id) => self
                .inv
                .host_name(id)
                .unwrap_or_else(|| "Unknown host".into()),
            Source::Device(route) => self
                .inv
                .device(route)
                .map(|device| device.record.display_name)
                .unwrap_or_else(|| "Unknown phone".into()),
        }
    }

    /// Run `f` on the Files pane with this id, if it is still open (in a tab or a workspace).
    fn with_files<R>(&self, id: i32, f: impl FnOnce(&FilesPane) -> R) -> Option<R> {
        self.with_pane(id, |pane, _| match pane {
            Pane::Files(pane) => Some(f(pane)),
            _ => None,
        })
        .flatten()
    }

    /// Redraw the pane if it is on screen.
    fn files_publish(&self, id: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        self.with_pane(id, |pane, place| {
            if let (Pane::Files(pane), true) = (pane, self.on_screen(place)) {
                self.present_files(&ui, pane, place, false);
            }
        });
    }

    /// The places the picker offers: this PC, connected hosts, available phones, and whatever
    /// a side is showing even if it has gone away since.
    fn files_sources(&self, pane: &FilesPane) -> Vec<Source> {
        let mut sources = vec![Source::Local];
        sources.extend(self.inv.connected_hosts().into_iter().map(Source::Host));
        sources.extend(self.inv.available_devices().into_iter().map(Source::Device));
        for side in &pane.sides {
            let source = side.borrow().source.clone();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        sources
    }

    fn source_label(&self, source: &Source) -> String {
        match source {
            Source::Local => "This PC".into(),
            Source::Host(id) => {
                let name = self.place_name(source);
                if self.inv.connected_hosts().contains(id) {
                    name
                } else {
                    format!("{name} (not connected)")
                }
            }
            Source::Device(route) => match self.inv.device(route) {
                Some(device) => {
                    let record = &device.record;
                    let reach = format!(
                        "{} > {}",
                        record.host_name,
                        transport_label(&record.connection_type)
                    );
                    if record.state == "available" {
                        format!("{} ({reach})", record.display_name)
                    } else {
                        format!("{} (not connected)", record.display_name)
                    }
                }
                None => "Unknown phone (not connected)".into(),
            },
        }
    }

    /// Whether the phone of a side (if it shows one) can be used right now.
    fn phone_ready(&self, source: &Source) -> bool {
        match source {
            Source::Device(route) => self
                .inv
                .device(route)
                .is_some_and(|device| device.record.state == "available"),
            _ => true,
        }
    }

    /// Draw a Files pane: both sides, their rows and what can be copied.
    pub fn present_files(
        &self,
        ui: &crate::MainWindow,
        pane: &FilesPane,
        place: Place,
        full: bool,
    ) {
        let sources = self.files_sources(pane);
        let labels: Vec<SharedString> = sources
            .iter()
            .map(|source| self.source_label(source).into())
            .collect();
        for index in 0..2 {
            let side = pane.sides[index].borrow();
            let other = pane.sides[1 - index].borrow();
            let selected: Vec<&Entry> = side
                .selected
                .iter()
                .filter_map(|row| side.entries.get(*row))
                .collect();
            let reason = copy_problem(
                side.source.kind(),
                other.source.kind(),
                &selected,
                other.loaded,
                &other.path,
                self.phone_ready(&side.source) && self.phone_ready(&other.source),
            );
            // A folder's own size says nothing about what is in it: only files count.
            let bytes: u64 = selected
                .iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.size)
                .sum();
            let count = side.entries.len();
            let mut summary = format!("{count} {}", if count == 1 { "item" } else { "items" });
            if !selected.is_empty() {
                summary.push_str(&format!(
                    ", {} selected ({})",
                    selected.len(),
                    size_text(bytes)
                ));
            }
            let place = self.place_name(&side.source);
            let top = side.source.kind() == Kind::Local && side.path.is_empty();
            let info = FileSide {
                place: place.as_str().into(),
                path: if top {
                    "Drives of this PC".into()
                } else {
                    side.path.as_str().into()
                },
                can_up: side.loaded && parent(side.source.kind(), &side.path).is_some(),
                loading: side.loading,
                loaded: side.loaded,
                error: side.error.as_str().into(),
                summary: if side.loaded {
                    summary.into()
                } else {
                    SharedString::new()
                },
                copy_reason: reason.into(),
                copy_target: format!(
                    "To {}: {}",
                    self.place_name(&other.source),
                    if other.path.is_empty() {
                        "no folder open"
                    } else {
                        &other.path
                    }
                )
                .into(),
                cursor: side.cursor.map(|row| row as i32).unwrap_or(-1),
                can_open: selected.len() == 1 && selected[0].opens(),
                source_index: sources
                    .iter()
                    .position(|source| *source == side.source)
                    .unwrap_or(0) as i32,
                input: side.input.as_str().into(),
                input_rev: side.input_rev,
            };
            if pane.info.row_data(index).as_ref() != Some(&info) {
                pane.info.set_row_data(index, info);
            }
        }
        *pane.sources.borrow_mut() = sources;
        let current: Vec<SharedString> = pane.labels.iter().collect();
        if current != labels {
            pane.labels.set_vec(labels);
        }
        // Everything above is a model of the pane: the view's data is only set when it starts
        // showing this pane.
        if full {
            self.panel_update(ui, place, true, |panel| {
                panel.kind = "files".into();
                panel.files = crate::FilesData {
                    id: pane.id,
                    sources: pane.labels.clone().into(),
                    sides: pane.info.clone().into(),
                    left_rows: pane.sides[0].borrow().rows.clone().into(),
                    right_rows: pane.sides[1].borrow().rows.clone().into(),
                };
            });
        }
    }

    /// Hosts or phones changed: the pickers and the copy buttons of every pane on screen
    /// follow.
    pub fn files_inventory_changed(&self) {
        let mut ids = Vec::new();
        self.each_pane(|pane, place| {
            if let (Pane::Files(pane), true) = (pane, self.on_screen(place)) {
                ids.push(pane.id);
            }
        });
        for id in ids {
            self.files_publish(id);
        }
    }

    /// Say what the folder field of a side should show.
    fn set_input(&self, id: i32, side: usize, text: &str) {
        self.with_files(id, |pane| pane.sides[side].borrow_mut().set_input(text));
        self.files_publish(id);
    }

    /// List `path` of `source` into a side. The rows on screen stay until the answer is in.
    fn files_load(self: &Rc<Self>, id: i32, side: usize, source: Source, path: String) {
        let request = self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            state.loading = true;
            state.error.clear();
            state.request += 1;
            state.request
        });
        let Some(request) = request else { return };
        self.files_publish(id);

        let backend = self.backend.clone();
        let place = source.clone();
        self.run(
            async move {
                match place {
                    Source::Local => tokio::task::spawn_blocking(move || list_local(&path))
                        .await
                        .unwrap_or_else(|_| Err("The folder could not be read.".into())),
                    Source::Host(host) => backend
                        .sftp_list(host, path)
                        .await
                        .map(|listing| {
                            let mut entries: Vec<Entry> = listing
                                .entries
                                .into_iter()
                                .filter(|entry| entry.name != "." && entry.name != "..")
                                .map(|entry| Entry {
                                    name: entry.name,
                                    is_dir: entry.is_dir,
                                    is_symlink: entry.is_symlink,
                                    is_drive: false,
                                    size: entry.size,
                                    modified: entry.modified.map(i64::from),
                                })
                                .collect();
                            sort_entries(&mut entries);
                            (listing.path, entries)
                        })
                        .map_err(|error| friendly(&error)),
                    Source::Device(route) => {
                        let shown = tidy_posix(&path);
                        backend
                            .device_files_list(route, path)
                            .await
                            .map(|list| {
                                let mut entries: Vec<Entry> = list
                                    .into_iter()
                                    .filter(|entry| entry.name != "." && entry.name != "..")
                                    .map(|entry| Entry {
                                        name: entry.name,
                                        is_dir: entry.is_dir,
                                        is_symlink: entry.is_symlink,
                                        is_drive: false,
                                        size: entry.size as u64,
                                        modified: Some(entry.mtime as i64),
                                    })
                                    .collect();
                                sort_entries(&mut entries);
                                (shown, entries)
                            })
                            .map_err(|error| friendly(&error))
                    }
                }
            },
            move |app, result| {
                let fresh = app.with_files(id, |pane| {
                    let mut state = pane.sides[side].borrow_mut();
                    if state.request != request || state.source != source {
                        return None;
                    }
                    state.loading = false;
                    match result {
                        Ok((path, entries)) => {
                            state.show(path, entries);
                            Some(state.path.clone())
                        }
                        Err(message) => {
                            state.error = message;
                            None
                        }
                    }
                });
                if let Some(Some(path)) = fresh {
                    app.set_input(id, side, &path);
                    // Listing a host that was disconnected connects it again: Home should say so.
                    if let Source::Host(host) = &source {
                        if !app.inv.connected_hosts().contains(host) {
                            app.reload();
                        }
                    }
                }
                app.files_publish(id);
            },
        );
    }

    /// (pane id, source, folder on screen) of a side of a pane.
    fn files_side(&self, id: i32, side: usize) -> Option<(i32, Source, String)> {
        self.with_files(id, |pane| {
            let state = pane.sides.get(side)?.borrow();
            Some((id, state.source.clone(), state.path.clone()))
        })
        .flatten()
    }

    fn files_pick_source(self: &Rc<Self>, id: i32, side: usize, index: i32) {
        let Some((id, current, _)) = self.files_side(id, side) else {
            return;
        };
        let picked = self
            .with_files(id, |pane| {
                pane.sources.borrow().get(index.max(0) as usize).cloned()
            })
            .flatten();
        let Some(source) = picked else { return };
        if source == current {
            return;
        }
        self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            let (rows, request) = (state.rows.clone(), state.request);
            rows.set_vec(Vec::new());
            let input_rev = state.input_rev.wrapping_add(1);
            *state = Side {
                rows,
                request,
                input_rev,
                ..Side::new(source.clone())
            };
        });
        self.files_retitle(id);
        let path = source.start_path();
        self.files_load(id, side, source, path);
    }

    /// Name the tab after the places it shows. A pane in a workspace panel has no tab.
    fn files_retitle(&self, id: i32) {
        let sessions = self.sessions.borrow();
        let Some(index) = sessions.iter().position(|pane| pane.id() == id) else {
            return;
        };
        let Pane::Files(pane) = &sessions[index] else {
            return;
        };
        let (left, right) = (
            pane.sides[0].borrow().source.clone(),
            pane.sides[1].borrow().source.clone(),
        );
        let title = tab_title(
            left.kind(),
            &self.place_name(&left),
            right.kind(),
            &self.place_name(&right),
        );
        // The tab carries the colour of the first machine that is not this PC.
        let hue = [&right, &left]
            .into_iter()
            .find_map(|source| match source {
                Source::Local => None,
                Source::Host(host) => Some(self.inv.hue(host)),
                Source::Device(route) => self
                    .inv
                    .device(route)
                    .map(|device| self.inv.hue(&device.record.host_id)),
            })
            .unwrap_or(0);
        if let Some(mut tab) = self.tabs.row_data(index) {
            if tab.title != title.as_str() || tab.hue != hue {
                tab.title = title.into();
                tab.hue = hue;
                self.tabs.set_row_data(index, tab);
            }
        }
    }

    fn files_go(self: &Rc<Self>, id: i32, side: usize, path: String) {
        let Some((id, source, _)) = self.files_side(id, side) else {
            return;
        };
        let path = match source.kind() {
            Kind::Local => tidy_local(&path),
            Kind::Host if path.trim().is_empty() => ".".into(),
            Kind::Device if path.trim().is_empty() => "/".into(),
            _ => path.trim().to_string(),
        };
        self.files_load(id, side, source, path);
    }

    fn files_up(self: &Rc<Self>, id: i32, side: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        if let Some(above) = parent(source.kind(), &path) {
            self.files_load(id, side, source, above);
        }
    }

    fn files_refresh(self: &Rc<Self>, id: i32, side: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        let loaded = self
            .with_files(id, |pane| pane.sides[side].borrow().loaded)
            .unwrap_or(false);
        let path = if loaded { path } else { source.start_path() };
        self.files_load(id, side, source, path);
    }

    fn files_open(self: &Rc<Self>, id: i32, side: usize, row: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        let name = self
            .with_files(id, |pane| {
                let state = pane.sides[side].borrow();
                state
                    .entries
                    .get(row)
                    .filter(|entry| entry.opens())
                    .map(|entry| entry.name.clone())
            })
            .flatten();
        if let Some(name) = name {
            let target = join(source.kind(), &path, &name);
            self.files_load(id, side, source, target);
        }
    }

    fn files_select(&self, id: i32, side: usize, row: usize, control: bool, shift: bool) {
        self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            if row >= state.entries.len() {
                return;
            }
            let mut selected = state.selected.clone();
            if shift {
                let from = state.anchor.unwrap_or(row);
                selected = (from.min(row)..=from.max(row)).collect();
            } else if control {
                if !selected.remove(&row) {
                    selected.insert(row);
                }
                state.anchor = Some(row);
            } else {
                selected = BTreeSet::from([row]);
                state.anchor = Some(row);
            }
            state.cursor = Some(row);
            state.set_selected(selected);
        });
        self.files_publish(id);
    }

    /// Keys of a file list: arrows, Enter, Backspace, Ctrl+A, F5.
    fn files_key(
        self: &Rc<Self>,
        id: i32,
        side: usize,
        text: &str,
        control: bool,
        shift: bool,
    ) -> bool {
        let is = |key: Key| text.chars().eq([char::from(key)]);
        let (count, cursor) = self
            .with_files(id, |pane| {
                let state = pane.sides[side].borrow();
                (state.entries.len(), state.cursor)
            })
            .unwrap_or((0, None));
        let step = |delta: i64| -> usize {
            let last = count.saturating_sub(1) as i64;
            match cursor {
                Some(row) => (row as i64 + delta).clamp(0, last) as usize,
                None if delta > 0 => 0,
                None => last as usize,
            }
        };
        let target = if is(Key::DownArrow) {
            Some(step(1))
        } else if is(Key::UpArrow) {
            Some(step(-1))
        } else if is(Key::PageDown) {
            Some(step(10))
        } else if is(Key::PageUp) {
            Some(step(-10))
        } else if is(Key::Home) {
            Some(0)
        } else if is(Key::End) {
            Some(count.saturating_sub(1))
        } else {
            None
        };
        if let Some(row) = target {
            if count > 0 {
                self.files_select(id, side, row, false, shift);
            }
            return true;
        }
        if is(Key::Return) {
            if let Some(row) = cursor {
                self.files_open(id, side, row);
            }
            return true;
        }
        if is(Key::Backspace) {
            self.files_up(id, side);
            return true;
        }
        if is(Key::F5) {
            self.files_refresh(id, side);
            return true;
        }
        if control && matches!(text, "a" | "A" | "\u{1}") {
            self.with_files(id, |pane| {
                let mut state = pane.sides[side].borrow_mut();
                let all = (0..state.entries.len()).collect();
                state.set_selected(all);
            });
            self.files_publish(id);
            return true;
        }
        false
    }

    /// Copy the selected files of `side` to the folder on the other side. A copy to or from a
    /// phone is confirmed first, naming the phone, its route and both folders.
    fn files_copy(self: &Rc<Self>, id: i32, side: usize) {
        if side > 1 {
            return;
        }
        let plan = self
            .with_files(id, |pane| {
                let from = pane.sides[side].borrow();
                let to = pane.sides[1 - side].borrow();
                let selected: Vec<&Entry> = from
                    .selected
                    .iter()
                    .filter_map(|row| from.entries.get(*row))
                    .collect();
                let problem = copy_problem(
                    from.source.kind(),
                    to.source.kind(),
                    &selected,
                    to.loaded,
                    &to.path,
                    self.phone_ready(&from.source) && self.phone_ready(&to.source),
                );
                if !problem.is_empty() {
                    return None;
                }
                let names: Vec<String> = selected.iter().map(|entry| entry.name.clone()).collect();
                Some((
                    from.source.clone(),
                    from.path.clone(),
                    to.source.clone(),
                    to.path.clone(),
                    names,
                ))
            })
            .flatten();
        let Some((from, from_dir, to, to_dir, names)) = plan else {
            return;
        };
        let (from_name, to_name) = (self.place_name(&from), self.place_name(&to));
        let phone = match (&from, &to) {
            (Source::Device(route), _) | (_, Source::Device(route)) => {
                let Some(device) = self.inv.device(route) else {
                    return;
                };
                Some(device)
            }
            _ => None,
        };
        let serial = phone
            .as_ref()
            .map(|device| device.record.serial.clone())
            .unwrap_or_default();

        let mut jobs = Vec::new();
        for name in &names {
            let path = join(from.kind(), &from_dir, name);
            let spec = match (&from, &to) {
                (Source::Local, Source::Host(host)) => JobSpec::SftpUpload {
                    host_id: host.clone(),
                    local_path: path,
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Host(host), Source::Local) => JobSpec::SftpDownload {
                    host_id: host.clone(),
                    remote_path: path,
                    local_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Local, Source::Device(route)) => JobSpec::DevicePush {
                    route_id: route.clone(),
                    serial: serial.clone(),
                    local_path: path,
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Device(route), Source::Local) => JobSpec::DevicePull {
                    route_id: route.clone(),
                    remote_path: path,
                    local_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Host(host), Source::Device(route)) => JobSpec::HostToDevice {
                    host_id: host.clone(),
                    remote_path: path,
                    route_id: route.clone(),
                    serial: serial.clone(),
                    device_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Device(route), Source::Host(host)) => JobSpec::DeviceToHost {
                    route_id: route.clone(),
                    device_path: path,
                    host_id: host.clone(),
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                _ => return,
            };
            jobs.push((copy_label(name, &from_name, &to_name), spec));
        }

        let Some(device) = phone else {
            for (label, spec) in jobs {
                self.job_start(label, spec);
            }
            return;
        };
        let mut facts = target_facts(&device);
        let files = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} files: {}", names.len(), names.join(", "))
        };
        facts.push(fact(
            if names.len() == 1 { "File" } else { "Files" },
            files,
            true,
        ));
        facts.push(fact("From", format!("{from_name}: {from_dir}"), true));
        facts.push(fact("To", format!("{to_name}: {to_dir}"), true));
        let to_phone = matches!(to, Source::Device(_));
        self.ask_with(
            Question {
                title: if to_phone {
                    format!("Copy to {}?", device.record.display_name)
                } else {
                    format!("Copy from {}?", device.record.display_name)
                },
                body: if to_phone {
                    "The files are written to this phone only. A file with the same name is not replaced without asking."
                } else {
                    "The files are read from this phone only. A file with the same name is not replaced without asking."
                }
                .into(),
                confirm: "Copy".into(),
                danger: false,
            },
            facts,
            "",
            Pending::CopyFiles(jobs),
        );
    }

    /// A copy ended well: show the new file wherever its folder is on screen.
    pub fn files_job_finished(self: &Rc<Self>, spec: &JobSpec) {
        let Some((source, dir)) = spec.destination() else {
            return;
        };
        let mut stale = Vec::new();
        self.each_pane(|pane, _| {
            let Pane::Files(pane) = pane else { return };
            for (side, state) in pane.sides.iter().enumerate() {
                let state = state.borrow();
                if state.loaded
                    && state.source == source
                    && same_dir(source.kind(), &state.path, &dir)
                {
                    stale.push((pane.id, side, state.path.clone()));
                }
            }
        });
        for (id, side, path) in stale {
            self.files_load(id, side, source.clone(), path);
        }
    }
}

/// What a Files tab is called: the places it shows that are not this PC.
fn tab_title(left: Kind, left_name: &str, right: Kind, right_name: &str) -> String {
    match (left, right) {
        (Kind::Local, Kind::Local) => "Files, this PC".to_string(),
        (Kind::Local, _) => format!("Files, {right_name}"),
        (_, Kind::Local) => format!("Files, {left_name}"),
        _ if left_name == right_name => format!("Files, {left_name}"),
        _ => format!("Files, {left_name} and {right_name}"),
    }
}

/// What the job list calls one copy.
fn copy_label(name: &str, from: &str, to: &str) -> String {
    format!("Copy {name} from {from} to {to}")
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

    #[test]
    fn which_copies_exist() {
        use Kind::*;
        for (from, to, ok) in [
            (Local, Host, true),
            (Host, Local, true),
            (Local, Device, true),
            (Device, Local, true),
            (Host, Device, true),
            (Device, Host, true),
            (Host, Host, false),
            (Device, Device, false),
            (Local, Local, false),
        ] {
            assert_eq!(pair_supported(from, to), ok, "{from:?} to {to:?}");
        }
    }

    fn file(name: &str) -> Entry {
        Entry {
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            is_drive: false,
            size: 10,
            modified: None,
        }
    }

    #[test]
    fn copy_reasons() {
        let (a, dir) = (
            file("a.txt"),
            Entry {
                is_dir: true,
                ..file("docs")
            },
        );
        assert_eq!(
            copy_problem(Kind::Local, Kind::Host, &[&a], true, "/tmp", true),
            ""
        );
        assert!(
            copy_problem(Kind::Host, Kind::Host, &[&a], true, "/tmp", true)
                .starts_with("Not supported yet")
        );
        assert!(
            copy_problem(Kind::Device, Kind::Device, &[&a], true, "/sdcard", true)
                .starts_with("Not supported yet")
        );
        assert_eq!(
            copy_problem(Kind::Local, Kind::Host, &[], true, "/tmp", true),
            "Select one or more files first."
        );
        assert!(
            copy_problem(Kind::Local, Kind::Host, &[&a, &dir], true, "/tmp", true)
                .starts_with("Folders cannot")
        );
        assert!(copy_problem(Kind::Host, Kind::Local, &[&a], true, "", true)
            .starts_with("Open a folder"));
        assert!(
            copy_problem(Kind::Host, Kind::Local, &[&a], false, "", true)
                .starts_with("The other side")
        );
        assert!(
            copy_problem(Kind::Local, Kind::Device, &[&a], true, "/sdcard", false)
                .starts_with("The phone must")
        );
    }

    #[test]
    fn tab_titles() {
        use Kind::*;
        assert_eq!(tab_title(Local, "This PC", Host, "fedora"), "Files, fedora");
        assert_eq!(tab_title(Device, "Pixel", Local, "This PC"), "Files, Pixel");
        assert_eq!(
            tab_title(Host, "fedora", Device, "Pixel"),
            "Files, fedora and Pixel"
        );
        assert_eq!(tab_title(Host, "fedora", Host, "fedora"), "Files, fedora");
        assert_eq!(
            tab_title(Local, "This PC", Local, "This PC"),
            "Files, this PC"
        );
    }

    #[test]
    fn folders_come_first() {
        let mut entries = vec![
            file("b.txt"),
            Entry {
                is_dir: true,
                ..file("zeta")
            },
            file("A.txt"),
            Entry {
                is_dir: true,
                ..file("Alpha")
            },
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta", "A.txt", "b.txt"]);
    }

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
