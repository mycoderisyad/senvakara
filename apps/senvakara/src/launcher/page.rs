//! Showing the New Tab page and the menu, and opening what the user picks from them.

use std::rc::Rc;

use senvakara_adapters_pty::ShellKind;
use slint::{ComponentHandle, Model, SharedString};

use crate::palette::Command;
use crate::{App, Launcher, Pane, TabInfo};

use super::items::launch_row;
use super::{step, Row};

impl App {
    pub fn init_launcher(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Launcher>();
        view.set_items(self.launcher.model.clone().into());
        view.set_quick(self.launcher.quick_model.clone().into());
        view.on_edited({
            let app = self.clone();
            move |text| app.launcher_show(&text, 0)
        });
        view.on_move({
            let app = self.clone();
            move |by| {
                let current = step(&app.launcher.rows.borrow(), app.launcher.current.get(), by);
                app.launcher.current.set(current);
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<Launcher>().set_current(current as i32);
                }
            }
        });
        view.on_run({
            let app = self.clone();
            move |index| app.launcher_run(index.max(0) as usize)
        });
        view.on_escape({
            let app = self.clone();
            move || app.launcher_escape()
        });
        view.on_quick_opening({
            let app = self.clone();
            move || app.launcher_quick_fill()
        });
        view.on_quick_run({
            let app = self.clone();
            move |index| app.launcher_quick_run(index.max(0) as usize)
        });
        self.launcher_detect();
    }

    /// Look for the shells installed on this PC, in the background. Home's WSL actions and
    /// the page follow when the answer arrives.
    pub(crate) fn launcher_detect(self: &Rc<Self>) {
        let backend = self.backend.clone();
        self.run(
            async move { backend.local_shells().await },
            |app, result| {
                // A failed look keeps the list there was.
                let Ok(shells) = result else { return };
                let distros = shells
                    .iter()
                    .filter_map(|shell| match &shell.kind {
                        ShellKind::Wsl { distro } => distro.clone(),
                        _ => None,
                    })
                    .collect();
                *app.launcher.shells.borrow_mut() = shells;
                app.inv.set_wsl(distros);
                // Redraws Home, which also brings the page up to date.
                app.reload();
            },
        );
    }

    /// The tab that holds the New Tab page, if it is open.
    fn launcher_tab(&self) -> Option<usize> {
        self.sessions
            .borrow()
            .iter()
            .position(|pane| matches!(pane, Pane::Launcher(_)))
    }

    /// "+" and Ctrl+Shift+T: show the New Tab page. There is one at most; asking again goes
    /// back to it and starts it over.
    pub(crate) fn launcher_open(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        self.launcher.launched.borrow_mut().take();
        ui.global::<Launcher>().set_note(SharedString::new());
        ui.global::<Launcher>().set_query(SharedString::new());
        self.launcher_show("", 0);
        let index = match self.launcher_tab() {
            Some(index) => index,
            None => {
                let id = self.take_id();
                self.sessions.borrow_mut().push(Pane::Launcher(id));
                self.tabs.push(TabInfo {
                    id,
                    kind: "new-tab".into(),
                    title: "New Tab".into(),
                    detail: SharedString::new(),
                    hue: 0,
                    exited: false,
                });
                self.tabs.row_count() - 1
            }
        };
        self.select(index as i32);
    }

    /// Draw the page for `query` with the line at or after `current` chosen.
    fn launcher_show(&self, query: &str, current: usize) {
        let Some(ui) = self.ui.upgrade() else { return };
        let rows = self.launcher_rows(query);
        let current = step(&rows, current, 0);
        self.launcher
            .model
            .set_vec(rows.iter().map(launch_row).collect::<Vec<_>>());
        *self.launcher.rows.borrow_mut() = rows;
        *self.launcher.query.borrow_mut() = query.to_string();
        self.launcher.current.set(current);
        ui.global::<Launcher>().set_current(current as i32);
    }

    /// Hosts, phones or shells changed: bring an open page up to date.
    pub(crate) fn launcher_refresh(&self) {
        if self.launcher_tab().is_none() {
            return;
        }
        let query = self.launcher.query.borrow().clone();
        self.launcher_show(&query, self.launcher.current.get());
        // A Remote Desktop that could not start says why on the page it was started from.
        let launched = self.launcher.launched.borrow().clone();
        let error = launched.and_then(|id| {
            self.inv
                .host_items()
                .into_iter()
                .find(|host| host.id == id.as_str())
                .map(|host| host.error)
        });
        if let (Some(error), Some(ui)) = (error.filter(|e| !e.is_empty()), self.ui.upgrade()) {
            ui.global::<Launcher>().set_note(error);
        }
    }

    fn launcher_run(self: &Rc<Self>, index: usize) {
        let row = self.launcher.rows.borrow().get(index).cloned();
        let Some(Row::Item(item)) = row else { return };
        self.launcher_start(item.entry.command, &item.entry.title);
    }

    /// Open `command`. When the New Tab page is the tab on screen, what opens takes its place:
    /// same position in the tab strip, no extra tab. Something that opens no tab (Remote
    /// Desktop runs in a window of its own; a host-key question is a dialog) leaves the page
    /// open with a note.
    fn launcher_start(self: &Rc<Self>, command: Command, title: &str) {
        let page = self
            .launcher_tab()
            .filter(|index| self.active.get() == *index as i32);
        let Some(page) = page else {
            self.run_command(command);
            return;
        };
        let note = match &command {
            Command::Host(host, action) if action == "rdp" => {
                *self.launcher.launched.borrow_mut() = Some(host.clone());
                format!("Opening {title} in Remote Desktop. It runs in a window of its own.")
            }
            _ => {
                self.launcher.launched.borrow_mut().take();
                String::new()
            }
        };
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Launcher>().set_note(note.into());
        }
        let before = self.sessions.borrow().len();
        self.run_command(command);
        let mut sessions = self.sessions.borrow_mut();
        let opened = sessions.len() == before + 1
            && matches!(sessions.get(page), Some(Pane::Launcher(_)))
            && page + 1 < sessions.len();
        if !opened {
            return;
        }
        // Every opener adds its tab at the end: move that one into the page's place.
        let last = sessions.len() - 1;
        let closed = sessions.swap_remove(page);
        drop(sessions);
        drop(closed);
        if let Some(tab) = self.tabs.row_data(last) {
            self.tabs.remove(last);
            self.tabs.set_row_data(page, tab);
        }
        self.select(page as i32);
    }

    /// Esc on the page: clear what was typed, or close the page when nothing was.
    fn launcher_escape(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !self.launcher.query.borrow().is_empty() {
            ui.global::<Launcher>().set_query(SharedString::new());
            self.launcher_show("", 0);
        } else if let Some(index) = self.launcher_tab() {
            self.close(index as i32);
        }
    }

    /// The menu next to "+" is about to open: list what it offers right now.
    fn launcher_quick_fill(&self) {
        let (items, view) = self.launcher_quick_items();
        self.launcher.quick_model.set_vec(view);
        *self.launcher.quick.borrow_mut() = items;
    }

    /// A click on the nth entry of the menu as it was shown.
    fn launcher_quick_run(self: &Rc<Self>, index: usize) {
        let item = self.launcher.quick.borrow().get(index).cloned();
        if let Some(item) = item {
            self.launcher_start(item.entry.command, &item.entry.title);
        }
    }

    /// Ctrl+Shift+1 to 9: open the nth entry the menu would show right now.
    pub(crate) fn launcher_quick_shortcut(self: &Rc<Self>, slot: usize) {
        self.launcher_quick_fill();
        self.launcher_quick_run(slot);
    }
}
