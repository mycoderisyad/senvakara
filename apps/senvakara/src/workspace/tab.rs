//! Workspace tabs: finding the one on screen, creating a new one and opening a saved one.

use std::rc::Rc;

use slint::{Model, SharedString};

use crate::{App, Pane, TabInfo};

use super::pane::WorkspacePane;
use super::stored::read_workspace;
use super::Layout;

impl App {
    /// The index of the workspace tab on screen.
    pub(super) fn workspace_tab(&self) -> Option<usize> {
        let index = self.active.get();
        let sessions = self.sessions.borrow();
        match (index >= 0).then(|| sessions.get(index as usize)).flatten() {
            Some(Pane::Workspace(_)) => Some(index as usize),
            _ => None,
        }
    }

    /// The tab that holds the workspace pane `id`.
    pub(super) fn workspace_tab_of(&self, id: i32) -> Option<usize> {
        self.sessions
            .borrow()
            .iter()
            .position(|pane| matches!(pane, Pane::Workspace(pane) if pane.id == id))
    }

    pub(super) fn with_workspace<R>(
        &self,
        tab: usize,
        f: impl FnOnce(&WorkspacePane) -> R,
    ) -> Option<R> {
        match self.sessions.borrow().get(tab) {
            Some(Pane::Workspace(pane)) => Some(f(pane)),
            _ => None,
        }
    }

    fn workspace_add_tab(self: &Rc<Self>, pane: WorkspacePane) {
        let (id, title) = (pane.id, pane.name.borrow().clone());
        self.sessions.borrow_mut().push(Pane::Workspace(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "workspace".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue: 0,
            exited: false,
        });
        let tab = self.tabs.row_count() - 1;
        self.workspace_sync(tab);
        self.select(tab as i32);
        self.workspaces_list_publish();
    }

    /// "New workspace": an unsaved tab with two empty panels.
    pub fn workspace_create(self: &Rc<Self>) {
        let id = self.take_id();
        self.workspace_add_tab(WorkspacePane::new(
            id,
            "New workspace",
            Layout::Columns,
            Vec::new(),
        ));
    }

    /// Open a saved workspace in a tab and start its sessions; show its tab if it is open.
    pub fn workspace_open(self: &Rc<Self>, saved_id: &str) {
        let open = self.sessions.borrow().iter().position(|pane| {
            matches!(pane, Pane::Workspace(pane) if pane.saved_id.borrow().as_deref() == Some(saved_id))
        });
        if let Some(tab) = open {
            self.select(tab as i32);
            return;
        }
        let record = self
            .ws
            .records
            .borrow()
            .iter()
            .find(|record| record.id == saved_id)
            .cloned();
        let Some(record) = record else { return };
        let (layout, specs) = read_workspace(&record);
        let pane = WorkspacePane::new(self.take_id(), &record.name, layout, specs.clone());
        *pane.saved_id.borrow_mut() = Some(record.id.clone());
        *pane.stored.borrow_mut() = Some((record.name.trim().to_string(), layout, specs));
        self.workspace_add_tab(pane);
    }
}
