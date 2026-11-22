//! Session tabs: opening, selecting and closing them, the Home sections and the window shortcuts.

use std::rc::Rc;

use senvakara_adapters_pty::ShellKind;
use slint::{ComponentHandle, Model, SharedString};

use crate::{
    desktop, notifier, term, App, Inventory, Pane, PanelView, Pending, Place, Question, Shell,
    TabInfo,
};

impl App {
    pub(crate) fn open_shell(self: &Rc<Self>, kind: &str, arg: &str) {
        let (shell, title) = match kind {
            "wsl" if arg.is_empty() => (ShellKind::Wsl { distro: None }, "WSL".to_string()),
            "wsl" => (
                ShellKind::Wsl {
                    distro: Some(arg.to_string()),
                },
                arg.to_string(),
            ),
            _ => (ShellKind::PowerShell, "PowerShell".to_string()),
        };
        self.open_local_shell(&shell, &title);
    }

    /// Open a terminal tab that runs `shell` on this PC.
    pub(crate) fn open_local_shell(self: &Rc<Self>, shell: &ShellKind, title: &str) {
        let id = self.take_id();
        let (cols, lines) = self.grid.get();
        let notify = notifier(id);
        let Some(ui) = self.ui.upgrade() else { return };
        match term::Session::spawn(id, shell, cols, lines, notify) {
            Ok(session) => {
                self.sessions.borrow_mut().push(Pane::Term(session));
                self.tabs.push(TabInfo {
                    id,
                    kind: "terminal".into(),
                    title: title.into(),
                    detail: SharedString::new(),
                    hue: if matches!(shell, ShellKind::Wsl { .. }) {
                        4
                    } else {
                        1
                    },
                    exited: false,
                });
                ui.global::<Inventory>().set_error(SharedString::new());
                self.select(self.tabs.row_count() as i32 - 1);
            }
            Err(error) => {
                ui.global::<Inventory>()
                    .set_error(format!("Could not start {title}: {error:#}").into());
                self.select(-1);
            }
        }
    }

    /// A session was put in a tab or a panel: start what needs its place to exist.
    pub fn pane_started(self: &Rc<Self>, id: i32) {
        let files = self
            .with_pane(id, |pane, place| {
                // A desktop only sends its screen while it is on screen.
                if let Pane::Desktop(pane) = pane {
                    pane.set_visible(self.on_screen(place));
                }
                matches!(pane, Pane::Files(_))
            })
            .unwrap_or(false);
        if files {
            self.files_start(id);
        }
    }

    pub fn select(self: &Rc<Self>, index: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let count = self.sessions.borrow().len() as i32;
        let index = if index >= count {
            count - 1
        } else {
            index.max(-1)
        };
        self.active.set(index);
        // A desktop is only asked for its screen while its tab is the one on screen.
        self.each_pane(|pane, place| {
            if let Pane::Desktop(pane) = pane {
                pane.set_visible(place.tab() as i32 == index);
            }
        });
        let kind = (index >= 0)
            .then(|| self.sessions.borrow().get(index as usize).map(Pane::kind))
            .flatten()
            .unwrap_or("");
        match kind {
            "" => ui.global::<Shell>().set_panel(PanelView::default()),
            "workspace" => {
                ui.global::<Shell>().set_panel(PanelView::default());
                self.present_workspace(&ui, index as usize);
            }
            "run" => {
                ui.global::<Shell>().set_panel(PanelView::default());
                if let Some(Pane::Run(pane)) = self.sessions.borrow().get(index as usize) {
                    pane.take();
                    self.present_run(&ui, pane, true);
                }
            }
            _ => self.present_place(&ui, Place::Tab(index as usize), true),
        }
        ui.global::<Shell>().set_active_kind(kind.into());
        ui.global::<Shell>().set_active_tab(index);
    }

    pub fn close(self: &Rc<Self>, index: i32) {
        if index < 0 || index as usize >= self.sessions.borrow().len() {
            return;
        }
        // Dropping the pane ends its session: the PTY, the SSH channel or the mirror.
        let closed = self.sessions.borrow_mut().remove(index as usize);
        let workspace = matches!(closed, Pane::Workspace(_));
        drop(closed);
        self.tabs.remove(index as usize);
        let active = self.active.get();
        let next = if active == index {
            index.min(self.sessions.borrow().len() as i32 - 1)
        } else if active > index {
            active - 1
        } else {
            active
        };
        self.select(next);
        if workspace {
            self.workspaces_reload();
        }
    }

    /// Close a tab, asking first when its session is still running.
    pub(crate) fn request_close(self: &Rc<Self>, index: i32) {
        let sessions = self.sessions.borrow();
        let Some(pane) = (index >= 0).then(|| sessions.get(index as usize)).flatten() else {
            return;
        };
        if !pane.running() {
            drop(sessions);
            self.close(index);
            return;
        }
        let title = self
            .tabs
            .row_data(index as usize)
            .map(|tab| tab.title)
            .unwrap_or_default();
        let (id, body) = (pane.id(), pane.closing_note());
        let confirm = match pane {
            Pane::Workspace(_) => "Close workspace",
            Pane::Run(_) => "Cancel run and close",
            _ => "Close session",
        };
        drop(sessions);
        self.ask(
            Question {
                title: format!("Close {title}?"),
                body,
                confirm: confirm.into(),
                danger: true,
            },
            Pending::CloseSession(id),
        );
    }

    /// Open a remote desktop tab on a connected host with an X11 session.
    pub(crate) fn open_desktop(self: &Rc<Self>, host: &senvakara_app::HostView) {
        if !host.connected {
            return;
        }
        let id = self.take_id();
        let hue = self.inv.hue(&host.id);
        let pane = desktop::DesktopPane::start(
            id,
            self.backend.clone(),
            host.id.clone(),
            host.display_name.clone(),
            hue,
            notifier(id),
        );
        self.sessions.borrow_mut().push(Pane::Desktop(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "desktop".into(),
            title: format!("{} desktop", host.display_name).into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }

    /// The terminal the copy and paste shortcuts act on: the one the user last used, if it is
    /// on screen, otherwise the terminal of the active tab.
    fn with_active<R>(&self, f: impl FnOnce(&term::Session) -> R) -> Option<R> {
        let index = self.active.get();
        if index < 0 {
            return None;
        }
        let focused = self.focus_term.get();
        let id = match self.locate(focused) {
            Some(place) if self.on_screen(place) => focused,
            _ => self.sessions.borrow().get(index as usize)?.id(),
        };
        self.with_term(id, f)
    }

    /// The user went to a section of Home: read what it shows.
    pub fn section_opened(self: &Rc<Self>, section: &str) {
        match section {
            "keychain" => self.keychain_reload(),
            "workspaces" => self.workspaces_reload(),
            "automations" => self.automations_opened(),
            "settings" => self.settings_opened(),
            _ => {}
        }
    }

    /// Window shortcuts: Ctrl+Tab, and Ctrl+Shift with T (the New Tab page), W, C, V, K (the
    /// command palette) and 1 to 9 (the nth entry of the menu next to "+").
    /// Plain Ctrl+T/W/K stay with the shell, as in Windows Terminal.
    pub(crate) fn shortcut(self: &Rc<Self>, text: &str, control: bool, shift: bool) -> bool {
        if !control {
            return false;
        }
        let key = text.chars().next().map(|c| c.to_ascii_lowercase());
        let tab =
            text.starts_with('\t') || text.starts_with(char::from(slint::platform::Key::Backtab));
        let count = self.sessions.borrow().len() as i32;
        if tab {
            // Home sits before the first session in the cycle.
            let step = if shift { -1 } else { 1 };
            let position = (self.active.get() + 1 + step).rem_euclid(count + 1);
            self.select(position - 1);
            return true;
        }
        if !shift {
            return false;
        }
        match key {
            Some('t') | Some('\u{14}') => {
                self.launcher_open();
                true
            }
            Some('w') | Some('\u{17}') => {
                let active = self.active.get();
                if active >= 0 {
                    self.request_close(active);
                }
                true
            }
            Some('k') | Some('\u{b}') => {
                self.palette_toggle();
                true
            }
            Some('c') | Some('\u{3}') => {
                if let Some(text) = self
                    .with_active(|session| session.selection_text())
                    .flatten()
                {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
                true
            }
            Some('v') | Some('\u{16}') => {
                let text = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
                if let Ok(text) = text {
                    self.with_active(|session| session.paste(&text));
                }
                true
            }
            Some(key) => match crate::launcher::quick_slot(key) {
                Some(slot) => {
                    self.launcher_quick_shortcut(slot);
                    true
                }
                None => false,
            },
            None => false,
        }
    }
}
