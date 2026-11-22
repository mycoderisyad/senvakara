//! The session model: what a tab holds, where a session is drawn and how `App` reaches one.

use crate::{desktop, files, logcat, mirror, run, term, workspace, App};

/// What a session tab holds. The first five are single sessions and can also sit in a panel
/// of a workspace.
// A handful of long-lived panes: boxing the large ones would only add a hop to every access.
#[allow(clippy::large_enum_variant)]
pub enum Pane {
    Term(term::Session),
    Mirror(mirror::MirrorPane),
    Logcat(logcat::LogcatPane),
    /// Two file lists side by side: this PC, a host or a phone on each.
    Files(files::FilesPane),
    /// The screen of a host's X11 session.
    Desktop(desktop::DesktopPane),
    /// Up to four sessions side by side.
    Workspace(workspace::WorkspacePane),
    /// A Flutter build-and-deploy or a recipe run.
    Run(run::RunPane),
    /// The New Tab page, by its id: a list of things to open, one of which takes its place.
    Launcher(i32),
}

impl Pane {
    pub fn id(&self) -> i32 {
        match self {
            Pane::Term(session) => session.id,
            Pane::Mirror(pane) => pane.id,
            Pane::Logcat(pane) => pane.id,
            Pane::Files(pane) => pane.id,
            Pane::Desktop(pane) => pane.id,
            Pane::Workspace(pane) => pane.id,
            Pane::Run(pane) => pane.id,
            Pane::Launcher(id) => *id,
        }
    }

    /// The view that draws it.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Pane::Term(_) => "terminal",
            Pane::Mirror(_) => "mirror",
            Pane::Logcat(_) => "logcat",
            Pane::Files(_) => "files",
            Pane::Desktop(_) => "desktop",
            Pane::Workspace(_) => "workspace",
            Pane::Run(_) => "run",
            Pane::Launcher(_) => "new-tab",
        }
    }

    /// Whether closing it would end something that is still going.
    pub fn running(&self) -> bool {
        match self {
            Pane::Term(session) => !session.exited(),
            Pane::Mirror(pane) => !pane.ended.get(),
            Pane::Logcat(pane) => pane.active(),
            // Copies belong to the job list and go on without the tab.
            Pane::Files(_) => false,
            Pane::Desktop(pane) => !pane.ended(),
            Pane::Workspace(pane) => pane.running(),
            Pane::Run(pane) => pane.running(),
            Pane::Launcher(_) => false,
        }
    }

    /// What the confirmation says will happen when it is closed.
    pub fn closing_note(&self) -> String {
        match self {
            Pane::Term(_) => "The shell and everything running in it will end.".into(),
            Pane::Mirror(_) => "Screen mirroring of this phone will stop.".into(),
            Pane::Logcat(_) => {
                "The log stream stops and the lines collected here are discarded.".into()
            }
            Pane::Files(_) => "Copies that are running continue in the activity list.".into(),
            Pane::Desktop(_) => {
                "Screen sharing stops on the host. Programs running on its desktop are not affected.".into()
            }
            Pane::Workspace(pane) => pane.closing_note(),
            Pane::Run(_) => {
                "The run is cancelled: it stops at the next point where it can. What it already did on the phone stays.".into()
            }
            Pane::Launcher(_) => String::new(),
        }
    }
}

/// Where a session is drawn: a tab of its own, or a panel (slot) of the workspace in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Tab(usize),
    Slot(usize, usize),
}

impl Place {
    pub fn tab(self) -> usize {
        match self {
            Place::Tab(tab) | Place::Slot(tab, _) => tab,
        }
    }
}

impl App {
    /// Where the session `id` is drawn.
    pub fn locate(&self, id: i32) -> Option<Place> {
        let sessions = self.sessions.borrow();
        sessions.iter().enumerate().find_map(|(tab, pane)| {
            if pane.id() == id {
                return Some(Place::Tab(tab));
            }
            match pane {
                Pane::Workspace(workspace) => {
                    workspace.slot_of(id).map(|slot| Place::Slot(tab, slot))
                }
                _ => None,
            }
        })
    }

    /// Run `f` on the pane at `place`. `f` must not open, close or move sessions.
    pub(crate) fn with_place<R>(&self, place: Place, f: impl FnOnce(&Pane) -> R) -> Option<R> {
        let sessions = self.sessions.borrow();
        match (place, sessions.get(place.tab())?) {
            (Place::Tab(_), pane) => Some(f(pane)),
            (Place::Slot(_, slot), Pane::Workspace(workspace)) => {
                let slots = workspace.slots.borrow();
                slots.get(slot)?.pane.as_ref().map(f)
            }
            _ => None,
        }
    }

    /// Run `f` on the session `id`, wherever it is drawn, with its place.
    pub fn with_pane<R>(&self, id: i32, f: impl FnOnce(&Pane, Place) -> R) -> Option<R> {
        let place = self.locate(id)?;
        self.with_place(place, |pane| f(pane, place))
    }

    /// Run `f` on every single session: the ones in tabs and the ones in workspace panels.
    pub fn each_pane(&self, mut f: impl FnMut(&Pane, Place)) {
        let sessions = self.sessions.borrow();
        for (tab, pane) in sessions.iter().enumerate() {
            match pane {
                Pane::Workspace(workspace) => {
                    for (slot, entry) in workspace.slots.borrow().iter().enumerate() {
                        if let Some(pane) = &entry.pane {
                            f(pane, Place::Slot(tab, slot));
                        }
                    }
                }
                pane => f(pane, Place::Tab(tab)),
            }
        }
    }

    /// Whether what is drawn at `place` is on screen.
    pub fn on_screen(&self, place: Place) -> bool {
        self.active.get() == place.tab() as i32
    }
}
