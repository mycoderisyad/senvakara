//! Draws a workspace tab: its panel rows, its toolbar and the picker.

use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{App, Place, SlotView, Workspaces};

use super::availability::blocked;
use super::labels::{panel_kind, panel_options, panel_title};
use super::naming::name_problem;
use super::{Shell, Spec};

impl App {
    /// The hue of the machine a panel runs on.
    fn spec_hue(&self, spec: &Spec) -> i32 {
        match spec {
            Spec::Terminal {
                shell: Shell::Ssh { host_id, .. },
                ..
            }
            | Spec::Desktop { host_id }
            | Spec::Files { host_id } => self.inv.hue(host_id),
            Spec::Terminal {
                shell: Shell::Wsl(_),
                ..
            } => 4,
            Spec::Terminal { .. } => 1,
            Spec::Mirror { route_id } | Spec::Logcat { route_id } => self
                .inv
                .device(route_id)
                .map(|device| self.inv.hue(&device.record.host_id))
                .unwrap_or(0),
            Spec::Unavailable { .. } => 0,
        }
    }

    /// Redraw a workspace: its panel rows always (they are its own model), and the toolbar and
    /// the picker when it is the tab on screen.
    pub fn workspace_publish(self: &Rc<Self>, tab: usize) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let options = panel_options(&hosts, &devices, &self.inv.wsl());
        let visible = self.active.get() == tab as i32;
        let mut fresh = Vec::new();
        let title = self.with_workspace(tab, |pane| {
            let slots = pane.slots.borrow();
            let rows: Vec<SlotView> = slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    let old = pane.view.row_data(index).unwrap_or_default();
                    let Some(spec) = &slot.spec else {
                        return SlotView {
                            state: "empty".into(),
                            choice: slot.choice.min(options.len().saturating_sub(1)) as i32,
                            ..SlotView::default()
                        };
                    };
                    let mut row = SlotView {
                        title: panel_title(spec, &hosts, &devices).into(),
                        kind: panel_kind(spec).into(),
                        hue: self.spec_hue(spec),
                        ..SlotView::default()
                    };
                    match &slot.pane {
                        Some(live) => {
                            row.state = "live".into();
                            // Keep what the session has drawn unless it is another session.
                            if old.state == "live" && panel_id(&old.panel) == live.id() {
                                row.panel = old.panel;
                            } else {
                                fresh.push(index);
                            }
                        }
                        None => {
                            let reason = blocked(spec, &hosts, &devices);
                            row.state = "blocked".into();
                            row.can_connect = reason.as_ref().is_some_and(|r| r.connect.is_some());
                            row.connecting = slot.connecting;
                            row.reason = reason
                                .map(|r| r.text)
                                .unwrap_or_else(|| "Starting.".into())
                                .into();
                        }
                    }
                    row
                })
                .collect();
            if pane.view.row_count() != rows.len() {
                pane.view.set_vec(rows);
            } else {
                for (index, row) in rows.into_iter().enumerate() {
                    if pane.view.row_data(index).as_ref() != Some(&row) {
                        pane.view.set_row_data(index, row);
                    }
                }
            }
            let name = pane.name.borrow().trim().to_string();
            if visible {
                let view = ui.global::<Workspaces>();
                let dirty = pane.dirty();
                let reason = name_problem(&name);
                let saved = pane.saved_id.borrow().is_some();
                view.set_name(pane.name.borrow().as_str().into());
                view.set_layout(pane.layout.get().key().into());
                view.set_slots(pane.view.clone().into());
                let labels: Vec<SharedString> =
                    options.iter().map(|(label, _)| label.into()).collect();
                let current: Vec<SharedString> = view.get_options().iter().collect();
                if current != labels {
                    view.set_options(ModelRc::new(VecModel::from(labels)));
                }
                view.set_dirty(dirty);
                view.set_save_label(
                    if !saved {
                        "Save workspace"
                    } else if dirty {
                        "Save changes"
                    } else {
                        "Saved"
                    }
                    .into(),
                );
                view.set_save_reason(
                    if !reason.is_empty() {
                        reason
                    } else if !dirty {
                        "There are no changes to save."
                    } else {
                        ""
                    }
                    .into(),
                );
                let message = pane.message.borrow();
                // A "Saved" note is stale as soon as something changes again.
                let show = !(message.0 && dirty);
                view.set_message(if show { message.1.as_str() } else { "" }.into());
                view.set_message_ok(message.0);
            }
            *pane.options.borrow_mut() = options;
            if name.is_empty() {
                "Workspace".to_string()
            } else {
                name
            }
        });
        if let (Some(title), Some(mut info)) = (title, self.tabs.row_data(tab)) {
            if info.title != title.as_str() {
                info.title = title.into();
                self.tabs.set_row_data(tab, info);
            }
        }
        // Sessions that just got a row draw themselves into it.
        if visible {
            for slot in fresh {
                self.present_place(&ui, Place::Slot(tab, slot), true);
            }
        }
    }

    /// The workspace tab came on screen: show its toolbar and redraw every live panel.
    pub fn present_workspace(self: &Rc<Self>, ui: &crate::MainWindow, tab: usize) {
        self.workspace_publish(tab);
        let live: Vec<usize> = self
            .with_workspace(tab, |pane| {
                pane.slots
                    .borrow()
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.pane.is_some())
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default();
        for slot in live {
            self.present_place(ui, Place::Slot(tab, slot), true);
        }
    }
}

/// The session a drawn panel belongs to.
fn panel_id(panel: &crate::PanelView) -> i32 {
    match panel.kind.as_str() {
        "terminal" => panel.term.id,
        "mirror" => panel.mirror.id,
        "logcat" => panel.logcat.id,
        "files" => panel.files.id,
        "desktop" => panel.desktop.id,
        _ => 0,
    }
}
