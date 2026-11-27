//! Drawing sessions into their views: the panel a view shows, a presenter per kind and refresh.

use std::rc::Rc;

use slint::{ComponentHandle, Model};

use crate::{
    desktop, logcat, mirror, term, App, LogcatData, MainWindow, Pane, PanelView, Place, Shell,
    TermData, TermLive,
};

impl App {
    /// What the view at `place` currently shows.
    fn panel_load(&self, ui: &MainWindow, place: Place) -> PanelView {
        match place {
            Place::Tab(_) => ui.global::<Shell>().get_panel(),
            Place::Slot(tab, slot) => match self.sessions.borrow().get(tab) {
                Some(Pane::Workspace(workspace)) => workspace
                    .view
                    .row_data(slot)
                    .map(|row| row.panel)
                    .unwrap_or_default(),
                _ => PanelView::default(),
            },
        }
    }

    fn panel_store(&self, ui: &MainWindow, place: Place, panel: PanelView) {
        match place {
            Place::Tab(_) => ui.global::<Shell>().set_panel(panel),
            Place::Slot(tab, slot) => {
                if let Some(Pane::Workspace(workspace)) = self.sessions.borrow().get(tab) {
                    if let Some(mut row) = workspace.view.row_data(slot) {
                        row.panel = panel;
                        workspace.view.set_row_data(slot, row);
                    }
                }
            }
        }
    }

    /// Change what the view at `place` shows. `fresh` starts from an empty panel, dropping
    /// whatever another session left there.
    pub fn panel_update(
        &self,
        ui: &MainWindow,
        place: Place,
        fresh: bool,
        change: impl FnOnce(&mut PanelView),
    ) {
        let mut panel = if fresh {
            PanelView::default()
        } else {
            self.panel_load(ui, place)
        };
        change(&mut panel);
        self.panel_store(ui, place, panel);
    }

    /// Draw the session at `place` into its view. `full` redraws everything (the view was
    /// showing something else, or nothing).
    pub fn present_place(&self, ui: &MainWindow, place: Place, full: bool) {
        self.with_place(place, |pane| match pane {
            Pane::Term(session) => self.present(ui, session, place, full),
            Pane::Mirror(pane) => self.present_mirror(ui, pane, place, full),
            Pane::Logcat(pane) => self.present_logcat(ui, pane, place, full),
            Pane::Files(pane) => self.present_files(ui, pane, place, full),
            Pane::Desktop(pane) => self.present_desktop(ui, pane, place, full),
            // The page draws the `Launcher` global: the panel only names its view.
            Pane::Launcher(_) => self.panel_update(ui, place, true, |panel| {
                panel.kind = "new-tab".into();
            }),
            Pane::Workspace(_) | Pane::Run(_) => {}
        });
    }

    /// Draw a desktop pane: the newest picture, the state and why it ended.
    fn present_desktop(
        &self,
        ui: &MainWindow,
        pane: &desktop::DesktopPane,
        place: Place,
        full: bool,
    ) {
        let update = pane.take();
        // After a switch the view may still hold another desktop's picture.
        let frame = update
            .frame
            .or_else(|| full.then(|| pane.last.borrow().clone()).flatten());
        let phase = pane.phase.borrow();
        let (tone, text, error) = match &*phase {
            desktop::Phase::Starting => ("busy", "Starting", ""),
            desktop::Phase::Connected => ("live", "Connected", ""),
            desktop::Phase::Ended(reason) => ("idle", "Ended", reason.as_str()),
        };
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "desktop".into();
            let view = &mut panel.desktop;
            view.id = pane.id;
            view.title = pane.title.as_str().into();
            view.view_only = pane.view_only.get();
            if let Some(frame) = frame {
                view.frame_width = frame.width() as i32;
                view.frame_height = frame.height() as i32;
                view.frame = slint::Image::from_rgba8(frame);
            }
            view.tone = tone.into();
            view.state_text = text.into();
            view.error = error.into();
            view.live = *phase == desktop::Phase::Connected;
            view.detail = pane.detail.borrow().as_str().into();
        });
    }

    /// Draw a terminal into its view. The rows and the cursor are models of the session, so
    /// the view's data is only set when it starts showing this session (`full`).
    fn present(&self, ui: &MainWindow, session: &term::Session, place: Place, full: bool) {
        let cursor = session.sync(full);
        session.set_live(TermLive {
            cursor_col: cursor.col,
            cursor_row: cursor.row,
            cursor_visible: cursor.visible,
            exited: session.exited(),
            scrollback: cursor.scrollback,
        });
        if full {
            self.panel_update(ui, place, true, |panel| {
                panel.kind = "terminal".into();
                panel.term = TermData {
                    id: session.id,
                    rows: session.rows(),
                    live: session.live(),
                };
            });
        }
    }

    /// Draw a mirror pane: the newest frame, its state and any error.
    fn present_mirror(&self, ui: &MainWindow, pane: &mirror::MirrorPane, place: Place, full: bool) {
        let update = pane.take();
        // After a switch the view may hold another phone's frame: show this one's last.
        let frame = update
            .frame
            .or_else(|| full.then(|| pane.last.borrow().clone()).flatten());
        let (width, _) = pane.size();
        let (tone, text) = if pane.ended.get() {
            ("idle", "Ended".to_string())
        } else if width == 0 {
            ("busy", "Starting".to_string())
        } else {
            ("live", format!("Live, {} fps", pane.fps.get()))
        };
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "mirror".into();
            let view = &mut panel.mirror;
            view.id = pane.id;
            view.title = pane.title.as_str().into();
            view.route = pane.route.as_str().into();
            view.hue = pane.hue;
            if let Some(frame) = frame {
                view.frame_width = frame.width() as i32;
                view.frame_height = frame.height() as i32;
                view.frame = slint::Image::from_rgba8(frame);
            }
            view.tone = tone.into();
            view.state_text = text.into();
            view.live = !pane.ended.get();
            view.error = pane.error.borrow().as_str().into();
        });
    }

    /// Draw a logcat pane: its lines are a model of their own, so only the words around them
    /// are set here. A new `revision` tells the view to scroll to the end.
    pub(crate) fn present_logcat(
        &self,
        ui: &MainWindow,
        pane: &logcat::LogcatPane,
        place: Place,
        full: bool,
    ) {
        let changed = pane.take();
        if (changed || full) && pane.follow.get() {
            pane.revision.set(pane.revision.get().wrapping_add(1));
        }
        let (tone, text) = pane.status();
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "logcat".into();
            panel.logcat = LogcatData {
                id: pane.id,
                lines: pane.model.clone().into(),
                title: pane.title.as_str().into(),
                route: pane.route.as_str().into(),
                hue: pane.hue,
                tone: tone.into(),
                state_text: text.into(),
                max_chars: pane.model.max_chars() as i32,
                empty_text: pane.empty_text().into(),
                active: pane.active(),
                error: pane.error.borrow().as_str().into(),
                filter: pane.filter.borrow().as_str().into(),
                follow: pane.follow.get(),
                revision: pane.revision.get(),
            };
        });
    }

    /// A session has new output, a new title or has ended.
    pub(crate) fn refresh(self: &Rc<Self>, id: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut title = None;
        let mut run_ended = false;
        let found = self.with_pane(id, |pane, place| {
            let visible = self.on_screen(place);
            match pane {
                Pane::Term(session) => {
                    title = session
                        .take_title()
                        .filter(|title| !title.trim().is_empty())
                        .map(|title| short_title(&title));
                    if visible {
                        self.present(&ui, session, place, false);
                    } else {
                        // Not visible: skip the drawing work. Showing it redraws it in full.
                        session.acknowledge();
                    }
                }
                Pane::Mirror(pane) => {
                    if visible {
                        self.present_mirror(&ui, pane, place, false);
                    } else {
                        // Drop frames nobody sees, but keep the ended state.
                        let _ = pane.take();
                    }
                }
                Pane::Logcat(pane) => {
                    if visible {
                        self.present_logcat(&ui, pane, place, false);
                    } else {
                        // Keep collecting: the newest lines are there when it is shown again.
                        let _ = pane.take();
                    }
                }
                // A file pane redraws itself when a listing arrives.
                Pane::Files(_) | Pane::Workspace(_) | Pane::Launcher(_) => {}
                Pane::Desktop(pane) => {
                    if visible {
                        self.present_desktop(&ui, pane, place, false);
                    } else {
                        // Keep the state (connected, ended and why); showing it draws it.
                        let _ = pane.take();
                    }
                }
                Pane::Run(pane) => {
                    // The run goes on whether or not its tab is on screen.
                    run_ended = pane.take();
                    if visible {
                        self.present_run(&ui, pane, false);
                    }
                }
            }
            (place, !pane.running())
        });
        // Only a tab of its own carries the session's title and its "ended" mark.
        if let Some((Place::Tab(index), exited)) = found {
            if let Some(mut tab) = self.tabs.row_data(index) {
                if title.is_some() || tab.exited != exited {
                    if let Some(title) = title {
                        tab.title = title.into();
                    }
                    tab.exited = exited;
                    self.tabs.set_row_data(index, tab);
                }
            }
        }
        if run_ended {
            self.workflows_run_ended();
        }
    }
}

/// Windows shells title the window with the full path of their executable
/// ("C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe"); show just the program.
/// Any other title (a prompt such as "user@host: ~/project") is kept as the program set it.
fn short_title(title: &str) -> String {
    let title = title.trim();
    match title.strip_suffix(".exe") {
        Some(path) => path.rsplit(['\\', '/']).next().unwrap_or(path).to_string(),
        None => title.to_string(),
    }
}
