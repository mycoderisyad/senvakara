//! The command palette (Ctrl+Shift+K): every "open ..." action the inventory offers right
//! now, plus saved workspaces and the Home sections, filtered by what the user types.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, Model, VecModel};

use crate::{App, Nav, Palette, PaletteItem};

/// What running an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A host action of the inventory: (host id, action id).
    Host(String, String),
    /// A device action of the inventory: (route id, action id).
    Device(String, String),
    Workspace(String),
    NewWorkspace,
    Section(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    /// Which machine or route it uses.
    pub detail: String,
    /// The icon: a pane kind, "workspace" or "section".
    pub kind: &'static str,
    pub command: Command,
}

/// The entries that match `query`, best first. Every word of the query must appear in the
/// title or the detail, ignoring case; titles that start with the query come first, then
/// titles that contain all of it, then matches through the detail. Ties keep their order.
pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return entries.to_vec();
    }
    let mut ranked: Vec<(u8, &Entry)> = entries
        .iter()
        .filter_map(|entry| {
            let title = entry.title.to_lowercase();
            let detail = entry.detail.to_lowercase();
            if !words
                .iter()
                .all(|word| title.contains(word) || detail.contains(word))
            {
                return None;
            }
            let rank = if title.starts_with(&query) {
                0
            } else if words.iter().all(|word| title.contains(word)) {
                1
            } else {
                2
            };
            Some((rank, entry))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, entry)| entry.clone()).collect()
}

/// Move the chosen row by `by` among `count` rows, stopping at both ends.
pub fn step(current: usize, by: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    (current as i64 + by as i64).clamp(0, count as i64 - 1) as usize
}

#[derive(Default)]
pub struct State {
    /// Everything that can be run, as it was when the palette opened.
    all: RefCell<Vec<Entry>>,
    shown: RefCell<Vec<Entry>>,
    current: Cell<usize>,
    model: Rc<VecModel<PaletteItem>>,
}

const SECTIONS: [(&str, &str); 5] = [
    ("hosts", "Hosts"),
    ("devices", "Devices"),
    ("workspaces", "Workspaces"),
    ("automations", "Automations"),
    ("settings", "Settings"),
];

impl App {
    pub fn init_palette(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Palette>();
        view.set_items(self.palette.model.clone().into());
        view.on_edited({
            let app = self.clone();
            move |text| app.palette_filter(&text)
        });
        view.on_move({
            let app = self.clone();
            move |by| {
                let count = app.palette.shown.borrow().len();
                app.palette
                    .current
                    .set(step(app.palette.current.get(), by, count));
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<Palette>()
                        .set_current(app.palette.current.get() as i32);
                }
            }
        });
        view.on_run({
            let app = self.clone();
            move |index| app.palette_run(index.max(0) as usize)
        });
        view.on_close({
            let app = self.clone();
            move || app.palette_close()
        });
    }

    /// Everything that can be opened right now. Actions the inventory marks as unavailable
    /// are left out: the palette only lists what will work.
    fn palette_entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for host in self.inv.host_items() {
            for action in host.actions.iter().filter(|a| a.reason.is_empty()) {
                let id = action.id.as_str();
                let (title, kind) = match id {
                    "terminal" if host.os == "windows" => {
                        ("Open PowerShell".to_string(), "terminal")
                    }
                    "terminal" => (format!("Open terminal on {}", host.name), "terminal"),
                    "tmux" => (
                        format!("Open terminal in tmux on {}", host.name),
                        "terminal",
                    ),
                    "files" => (format!("Open files on {}", host.name), "files"),
                    "desktop" => (format!("Open desktop of {}", host.name), "desktop"),
                    wsl if wsl.starts_with("wsl:") => {
                        (format!("Open WSL {}", &wsl[4..]), "terminal")
                    }
                    _ => continue,
                };
                entries.push(Entry {
                    title,
                    detail: host.subtitle.to_string(),
                    kind,
                    command: Command::Host(host.id.to_string(), id.to_string()),
                });
            }
        }
        for device in self.inv.device_items() {
            for action in device.actions.iter().filter(|a| a.reason.is_empty()) {
                let id = action.id.as_str();
                let (title, kind) = match id {
                    "mirror" => (format!("Open screen of {}", device.name), "mirror"),
                    "logcat" => (format!("Open logcat of {}", device.name), "logcat"),
                    "files" => (format!("Open files on {}", device.name), "files"),
                    _ => continue,
                };
                entries.push(Entry {
                    title,
                    detail: device.route.to_string(),
                    kind,
                    command: Command::Device(device.id.to_string(), id.to_string()),
                });
            }
        }
        for (id, name) in self.ws.names() {
            entries.push(Entry {
                title: format!("Open workspace {name}"),
                detail: "Saved workspace".into(),
                kind: "workspace",
                command: Command::Workspace(id),
            });
        }
        entries.push(Entry {
            title: "New workspace".into(),
            detail: "An empty layout of two panels".into(),
            kind: "workspace",
            command: Command::NewWorkspace,
        });
        for (id, label) in SECTIONS {
            entries.push(Entry {
                title: format!("Go to {label}"),
                detail: "Section of Home".into(),
                kind: "section",
                command: Command::Section(id),
            });
        }
        entries
    }

    /// Open the palette, or close it when it is open.
    pub fn palette_toggle(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Palette>();
        if view.get_open() {
            self.palette_close();
            return;
        }
        *self.palette.all.borrow_mut() = self.palette_entries();
        view.set_query(slint::SharedString::new());
        self.palette_filter("");
        view.set_open(true);
    }

    fn palette_close(&self) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Palette>().set_open(false);
        }
    }

    fn palette_filter(&self, query: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let shown = filter(&self.palette.all.borrow(), query);
        self.palette.model.set_vec(
            shown
                .iter()
                .map(|entry| PaletteItem {
                    title: entry.title.as_str().into(),
                    detail: entry.detail.as_str().into(),
                    kind: entry.kind.into(),
                })
                .collect::<Vec<_>>(),
        );
        *self.palette.shown.borrow_mut() = shown;
        self.palette.current.set(0);
        ui.global::<Palette>().set_current(0);
    }

    fn palette_run(self: &Rc<Self>, index: usize) {
        let entry = self.palette.shown.borrow().get(index).cloned();
        let Some(entry) = entry else { return };
        self.palette_close();
        match entry.command {
            Command::Host(host, action) => self.host_action(&host, &action),
            Command::Device(route, action) => self.device_action(&route, &action),
            Command::Workspace(id) => self.workspace_open(&id),
            Command::NewWorkspace => self.workspace_create(),
            Command::Section(section) => self.go_to_section(section),
        }
    }

    /// Show Home on one of its sections.
    pub fn go_to_section(self: &Rc<Self>, section: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.global::<Nav>().set_section(section.into());
        self.select(-1);
        self.section_opened(section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, detail: &str) -> Entry {
        Entry {
            title: title.into(),
            detail: detail.into(),
            kind: "terminal",
            command: Command::NewWorkspace,
        }
    }

    fn titles(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.title.as_str()).collect()
    }

    fn sample() -> Vec<Entry> {
        vec![
            entry("Open PowerShell", "Windows, this computer"),
            entry("Open terminal on Fedora laptop", "ssh, me@192.0.2.10"),
            entry("Open files on Fedora laptop", "ssh, me@192.0.2.10"),
            entry("Open screen of Redmi", "RAFGT  >  USB"),
            entry("Open workspace Bench", "Saved workspace"),
            entry("Go to Settings", "Section of Home"),
        ]
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let all = sample();
        assert_eq!(filter(&all, ""), all);
        assert_eq!(filter(&all, "   "), all);
    }

    #[test]
    fn matching_ignores_case_and_needs_every_word() {
        let all = sample();
        assert_eq!(
            titles(&filter(&all, "FEDORA")),
            [
                "Open terminal on Fedora laptop",
                "Open files on Fedora laptop"
            ]
        );
        assert_eq!(
            titles(&filter(&all, "files fedora")),
            ["Open files on Fedora laptop"]
        );
        assert!(filter(&all, "fedora screen").is_empty());
        assert!(filter(&all, "zzz").is_empty());
    }

    #[test]
    fn the_detail_matches_too_but_ranks_after_titles() {
        let all = vec![
            entry("Open screen of Redmi", "Bench host  >  USB"),
            entry("Open workspace Bench", "Saved workspace"),
            entry("Bench notes", "x"),
        ];
        assert_eq!(
            titles(&filter(&all, "bench")),
            [
                "Bench notes",
                "Open workspace Bench",
                "Open screen of Redmi"
            ]
        );
        assert_eq!(titles(&filter(&all, "usb")), ["Open screen of Redmi"]);
    }

    #[test]
    fn a_title_that_starts_with_the_query_comes_first() {
        let all = sample();
        assert_eq!(filter(&all, "go")[0].title, "Go to Settings");
        assert_eq!(filter(&all, "open s")[0].title, "Open screen of Redmi");
    }

    #[test]
    fn the_chosen_row_stops_at_both_ends() {
        assert_eq!(step(0, -1, 5), 0);
        assert_eq!(step(0, 1, 5), 1);
        assert_eq!(step(4, 1, 5), 4);
        assert_eq!(step(2, 10, 5), 4);
        assert_eq!(step(3, 1, 0), 0);
    }
}
