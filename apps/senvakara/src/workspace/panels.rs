//! What the user does to the panels of the workspace on screen: add, close, connect, and
//! change the layout.

use std::rc::Rc;

use slint::Model;

use crate::inventory::{fact, friendly};
use crate::{App, Pane, Pending, Question};

use super::availability::blocked;
use super::naming::{now_millis, tmux_name};
use super::pane::Slot;
use super::{Layout, Shell, Spec};

impl App {
    pub(super) fn workspace_add(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        self.with_workspace(tab, |pane| {
            let mut slots = pane.slots.borrow_mut();
            let Some(slot) = slots.get_mut(index) else {
                return;
            };
            if slot.spec.is_some() {
                return;
            }
            let Some((_, spec)) = pane.options.borrow().get(slot.choice).cloned() else {
                return;
            };
            slot.spec = Some(match spec {
                Spec::Terminal {
                    title,
                    shell:
                        Shell::Ssh {
                            host_id,
                            tmux: Some(_),
                        },
                } => Spec::Terminal {
                    title,
                    shell: Shell::Ssh {
                        host_id,
                        tmux: Some(tmux_name(now_millis())),
                    },
                },
                other => other,
            });
            slot.choice = 0;
        });
        self.workspace_sync(tab);
    }

    pub(super) fn workspace_close_ask(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let found = self
            .with_workspace(tab, |pane| {
                let slots = pane.slots.borrow();
                let slot = slots.get(index)?;
                let running = slot.pane.as_ref().is_some_and(Pane::running);
                let note = slot
                    .pane
                    .as_ref()
                    .map(Pane::closing_note)
                    .unwrap_or_default();
                Some((pane.id, running, note))
            })
            .flatten();
        let Some((id, running, note)) = found else {
            return;
        };
        if !running {
            self.workspace_clear(id, index);
            return;
        }
        let title = self
            .with_workspace(tab, |pane| pane.view.row_data(index))
            .flatten()
            .map(|row| row.title.to_string())
            .unwrap_or_default();
        self.ask(
            Question {
                title: format!("Close {title}?"),
                body: format!("{note} The panel becomes empty; save the workspace to keep that."),
                confirm: "Close panel".into(),
                danger: true,
            },
            Pending::CloseWorkspacePanel {
                workspace: id,
                slot: index,
            },
        );
    }

    /// Empty a panel of the workspace pane `id`, ending its session.
    pub fn workspace_clear(self: &Rc<Self>, id: i32, index: usize) {
        let Some(tab) = self.workspace_tab_of(id) else {
            return;
        };
        let old = self
            .with_workspace(tab, |pane| {
                pane.slots.borrow_mut().get_mut(index).map(std::mem::take)
            })
            .flatten();
        // Dropped after the borrow ends: ending a session may ask for a redraw.
        drop(old);
        self.workspace_publish(tab);
    }

    pub(super) fn workspace_layout_ask(self: &Rc<Self>, tab: usize, layout: Layout) {
        let found = self.with_workspace(tab, |pane| {
            let lost: Vec<usize> = pane
                .slots
                .borrow()
                .iter()
                .enumerate()
                .skip(layout.slots())
                .filter(|(_, slot)| slot.spec.is_some())
                .map(|(index, _)| index)
                .collect();
            (pane.id, pane.layout.get(), lost)
        });
        let Some((id, current, lost)) = found else {
            return;
        };
        if current == layout {
            return;
        }
        if lost.is_empty() {
            self.workspace_set_layout(id, layout);
            return;
        }
        let titles: Vec<String> = self
            .with_workspace(tab, |pane| {
                lost.iter()
                    .filter_map(|index| pane.view.row_data(*index))
                    .map(|row| row.title.to_string())
                    .collect()
            })
            .unwrap_or_default();
        self.ask_with(
            Question {
                title: format!("Switch to {}?", layout.label()),
                body: "The panels that do not fit are closed and their sessions end.".into(),
                confirm: "Switch layout".into(),
                danger: true,
            },
            vec![fact("Closed", titles.join(", "), false)],
            "",
            Pending::WorkspaceLayout {
                workspace: id,
                layout,
            },
        );
    }

    pub fn workspace_set_layout(self: &Rc<Self>, id: i32, layout: Layout) {
        let Some(tab) = self.workspace_tab_of(id) else {
            return;
        };
        let dropped = self.with_workspace(tab, |pane| {
            pane.layout.set(layout);
            let mut slots = pane.slots.borrow_mut();
            let rest: Vec<Slot> = if slots.len() > layout.slots() {
                slots.drain(layout.slots()..).collect()
            } else {
                Vec::new()
            };
            slots.resize_with(layout.slots(), Slot::default);
            rest
        });
        drop(dropped);
        self.workspace_sync(tab);
    }

    pub(super) fn workspace_connect(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let found = self
            .with_workspace(tab, |pane| {
                let mut slots = pane.slots.borrow_mut();
                let slot = slots.get_mut(index)?;
                let host = blocked(slot.spec.as_ref()?, &hosts, &devices)?.connect?;
                slot.connecting = true;
                Some((pane.id, host))
            })
            .flatten();
        let Some((id, host_id)) = found else { return };
        self.workspace_publish(tab);
        let backend = self.backend.clone();
        self.run(
            async move { backend.host_connect(host_id).await },
            move |app, result| {
                let Some(tab) = app.workspace_tab_of(id) else {
                    return;
                };
                app.with_workspace(tab, |pane| {
                    if let Some(slot) = pane.slots.borrow_mut().get_mut(index) {
                        slot.connecting = false;
                    }
                    *pane.message.borrow_mut() = match &result {
                        Ok(_) => (true, String::new()),
                        Err(error) => (false, friendly(error)),
                    };
                });
                // Reloading the inventory starts every panel that can run now.
                app.reload();
                app.workspace_publish(tab);
            },
        );
    }
}
