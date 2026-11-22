//! The saved workspaces: the list in the Workspaces section, deleting and saving.

use std::cell::RefCell;
use std::rc::Rc;

use senvakara_app::{WorkspaceRecord, WorkspaceSave};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::inventory::{fact, friendly};
use crate::{App, Pane, Pending, Question, WorkspaceItem, Workspaces};

use super::labels::summary;
use super::naming::name_problem;
use super::stored::{read_workspace, write_workspace};

/// The Workspaces section: what is saved.
#[derive(Default)]
pub struct State {
    pub(super) records: RefCell<Vec<WorkspaceRecord>>,
    pub(super) items: Rc<VecModel<WorkspaceItem>>,
}

impl State {
    /// (id, name) of every saved workspace, for the command palette.
    pub fn names(&self) -> Vec<(String, String)> {
        self.records
            .borrow()
            .iter()
            .map(|record| (record.id.clone(), record.name.clone()))
            .collect()
    }
}

impl App {
    /// Read the saved workspaces again and redraw the section.
    pub fn workspaces_reload(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Workspaces>();
        match self.backend.workspaces_list() {
            Ok(records) => {
                *self.ws.records.borrow_mut() = records;
                view.set_error(SharedString::new());
            }
            Err(error) => view.set_error(friendly(&error).into()),
        }
        self.workspaces_list_publish();
    }

    pub(super) fn workspaces_list_publish(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let open: Vec<String> = self
            .sessions
            .borrow()
            .iter()
            .filter_map(|pane| match pane {
                Pane::Workspace(pane) => pane.saved_id.borrow().clone(),
                _ => None,
            })
            .collect();
        let items: Vec<WorkspaceItem> = self
            .ws
            .records
            .borrow()
            .iter()
            .map(|record| {
                let (layout, slots) = read_workspace(record);
                WorkspaceItem {
                    id: record.id.as_str().into(),
                    name: record.name.as_str().into(),
                    layout_text: layout.label().into(),
                    summary: summary(&slots, &hosts, &devices).into(),
                    open: open.contains(&record.id),
                }
            })
            .collect();
        let current: Vec<WorkspaceItem> = self.ws.items.iter().collect();
        if current != items {
            ui.global::<crate::Nav>()
                .set_workspace_count(items.len().to_string().into());
            self.ws.items.set_vec(items);
        }
    }

    pub(super) fn workspace_remove_ask(&self, saved_id: &str) {
        let record = self
            .ws
            .records
            .borrow()
            .iter()
            .find(|record| record.id == saved_id)
            .cloned();
        let Some(record) = record else { return };
        let (layout, slots) = read_workspace(&record);
        let facts = vec![
            fact("Workspace", record.name.as_str(), false),
            fact("Layout", layout.label(), false),
            fact(
                "Panels",
                summary(&slots, &self.inv.hosts(), &self.inv.devices()),
                false,
            ),
        ];
        self.ask_with(
            Question {
                title: format!("Delete {}?", record.name),
                body: "The saved layout is removed. If it is open, its tab is closed and its sessions end. Hosts and phones are not affected.".into(),
                confirm: "Delete workspace".into(),
                danger: true,
            },
            facts,
            "",
            Pending::DeleteWorkspace(record.id),
        );
    }

    /// The user confirmed deleting a saved workspace.
    pub fn workspace_delete(self: &Rc<Self>, saved_id: String) {
        let open = self.sessions.borrow().iter().position(|pane| {
            matches!(pane, Pane::Workspace(pane) if pane.saved_id.borrow().as_deref() == Some(saved_id.as_str()))
        });
        if let Err(error) = self.backend.workspace_delete(saved_id) {
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<Workspaces>().set_error(friendly(&error).into());
            }
            return;
        }
        if let Some(tab) = open {
            self.close(tab as i32);
        }
        self.workspaces_reload();
    }

    pub(super) fn workspace_save(self: &Rc<Self>) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let input = self.with_workspace(tab, |pane| {
            let name = pane.name.borrow().trim().to_string();
            if !name_problem(&name).is_empty() {
                return None;
            }
            let (layout, panels) = write_workspace(pane.layout.get(), &pane.specs());
            Some(WorkspaceSave {
                id: pane.saved_id.borrow().clone(),
                name,
                layout,
                panels,
            })
        });
        let Some(Some(input)) = input else { return };
        let result = self.backend.workspace_save(input);
        self.with_workspace(tab, |pane| match &result {
            Ok(record) => {
                *pane.saved_id.borrow_mut() = Some(record.id.clone());
                *pane.stored.borrow_mut() = Some((
                    record.name.trim().to_string(),
                    pane.layout.get(),
                    pane.specs(),
                ));
                *pane.message.borrow_mut() = (true, format!("Saved {}.", record.name));
            }
            Err(error) => {
                *pane.message.borrow_mut() = (
                    false,
                    crate::inventory::friendly_with(
                        error,
                        "This workspace was deleted in the meantime.",
                    ),
                );
            }
        });
        self.workspace_publish(tab);
        self.workspaces_reload();
    }
}
