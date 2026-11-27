//! Loading, picking, creating, renaming and deleting workflows, and saving the open one a
//! moment after each change (on the backend runtime, so typing never waits for the disk).

use std::rc::Rc;
use std::time::Duration;

use senvakara_app::workflow::Viewport;
use senvakara_app::Workflow;
use slint::{Model, SharedString};

use super::geometry::View;
use super::view::{Machine, Machines};
use super::Open;
use crate::run::{friendly_raw, run_state, when_text};
use crate::{App, Pending, Question, WfRun};

/// How long after the last change the workflow is written.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Past runs listed in the workflow panel.
const RECENT_RUNS: usize = 5;

impl App {
    /// Reads the saved workflows and opens `keep` (an id), else the one already open, else the
    /// first. Unsaved changes of the open workflow are written first.
    pub(super) fn workflows_reload(self: &Rc<Self>, keep: Option<String>) {
        self.workflows_flush();
        self.workflows_read_machines();
        let list = match self.backend.workflows_list() {
            Ok(list) => list,
            Err(error) => {
                self.workflows_error(&crate::inventory::friendly(&error));
                return;
            }
        };
        let want = keep.or_else(|| self.wf.open.borrow().as_ref().map(|o| o.id.clone()));
        *self.wf.list.borrow_mut() = list
            .iter()
            .map(|w| (w.id.clone(), w.name.clone()))
            .collect();
        self.wf.names.set_vec(
            list.iter()
                .map(|w| SharedString::from(w.name.as_str()))
                .collect::<Vec<_>>(),
        );
        let pick = want
            .and_then(|id| list.iter().find(|w| w.id == id))
            .or(list.first());
        let was = self.wf.open.borrow().as_ref().map(|o| o.id.clone());
        if pick.map(|w| &w.id) != was.as_ref() {
            self.wf.selected.borrow_mut().clear();
            self.wf.script.set(0);
            self.wf.health.borrow_mut().clear();
        }
        *self.wf.open.borrow_mut() = pick.map(|w| {
            let v = w.graph.viewport;
            self.wf.view.set(View {
                pan_x: v.x,
                pan_y: v.y,
                zoom: v.zoom.clamp(0.5, 2.0),
            });
            self.with_view(|v| v.set_save_state("Saved on this PC.".into()));
            Open {
                id: w.id.clone(),
                name: w.name.clone(),
                graph: w.graph.clone(),
            }
        });
        // History first: the cards show how their steps went in the last run.
        self.workflows_runs_reload();
        self.workflows_postures_load();
        self.workflows_refresh(true);
        self.workflows_postures_auto(false);
        self.workflows_health_auto(false);
    }

    /// The latest runs of the open workflow, for the workflow panel. Called on open and
    /// whenever a run tab ends.
    pub(crate) fn workflows_runs_reload(&self) {
        let Some(id) = self.wf.open.borrow().as_ref().map(|o| o.id.clone()) else {
            self.wf.runs.set_vec(Vec::new());
            self.wf.last_run.borrow_mut().clear();
            return;
        };
        let runs = self.backend.flow_history().unwrap_or_default();
        *self.wf.last_run.borrow_mut() = runs
            .iter()
            .find(|r| r.workflow_id.as_deref() == Some(id.as_str()))
            .map(|r| {
                r.steps
                    .iter()
                    .filter_map(|s| Some((s.node_id.clone()?, (s.status.clone(), s.changed))))
                    .collect()
            })
            .unwrap_or_default();
        let rows: Vec<WfRun> = runs
            .iter()
            .filter(|r| r.workflow_id.as_deref() == Some(id.as_str()))
            .take(RECENT_RUNS)
            .map(|r| {
                let (tone, word) = run_state(&r.status);
                let result = r
                    .summary
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                WfRun {
                    when: when_text(&r.created_at).into(),
                    tone: tone.into(),
                    state_text: word.into(),
                    result: friendly_raw(result).into(),
                }
            })
            .collect();
        self.wf.runs.set_vec(rows);
    }

    /// A run tab ended: new history, and the cards show how their steps went.
    pub(crate) fn workflows_run_ended(self: &Rc<Self>) {
        // A run may have moved a host's SSH port in Hosts (plan WF §4).
        self.reload();
        self.workflows_runs_reload();
        self.workflows_refresh(false);
        // A run may have changed what the posture check sees.
        self.workflows_postures_auto(true);
        self.workflows_health_auto(true);
    }

    /// Hosts or phones changed in the inventory: names and states on the cards follow.
    pub(crate) fn workflows_inventory_changed(self: &Rc<Self>) {
        if self.wf.open.borrow().is_none() {
            return;
        }
        self.workflows_read_machines();
        self.workflows_refresh(false);
    }

    /// Hosts and devices from the inventory, for anchor names, colours and state.
    fn workflows_read_machines(&self) {
        let mut machines = Machines::default();
        for host in self.inv.hosts() {
            if host.is_local || host.ssh.is_none() {
                continue;
            }
            let state = if host.connected {
                "Connected"
            } else if host.reconnecting.is_some() {
                "Reconnecting"
            } else {
                "Not connected"
            };
            machines.hosts.insert(
                host.id.clone(),
                Machine {
                    name: host.display_name.clone(),
                    hue: self.inv.hue(&host.id),
                    reachable: host.connected,
                    state: state.into(),
                },
            );
        }
        for device in self.inv.devices() {
            let r = &device.record;
            let available = r.state == "available";
            let state = match r.state.as_str() {
                "available" => "Available",
                "unauthorized" => "Not authorized",
                "offline" => "Offline",
                _ => "Not connected",
            };
            machines.devices.insert(
                r.route_id.clone(),
                Machine {
                    name: format!("{} on {}", r.display_name, r.host_name),
                    hue: self.inv.hue(&r.host_id),
                    reachable: available,
                    state: state.into(),
                },
            );
        }
        *self.wf.machines.borrow_mut() = machines;
    }

    pub(super) fn workflows_pick(self: &Rc<Self>, index: i32) {
        let id = self
            .wf
            .list
            .borrow()
            .get(index.max(0) as usize)
            .map(|(id, _)| id.clone());
        if id.is_some() {
            self.workflows_reload(id);
        }
    }

    pub(super) fn workflows_create(self: &Rc<Self>) {
        self.workflows_flush();
        let taken: Vec<String> = self
            .wf
            .list
            .borrow()
            .iter()
            .map(|(_, n)| n.clone())
            .collect();
        let name = (1..)
            .map(|i| {
                if i == 1 {
                    "New workflow".to_string()
                } else {
                    format!("New workflow {i}")
                }
            })
            .find(|n| !taken.contains(n))
            .expect("a free name");
        match self.backend.workflow_save(None, name, &Workflow::default()) {
            Ok(saved) => self.workflows_reload(Some(saved.id)),
            Err(error) => self.workflows_error(&crate::inventory::friendly(&error)),
        }
    }

    pub(super) fn workflows_rename(self: &Rc<Self>, name: &str) {
        let Some(id) = self.wf.open.borrow().as_ref().map(|o| o.id.clone()) else {
            return;
        };
        if name.trim().is_empty() {
            self.with_view(|w| {
                w.set_save_state("A workflow needs a name; the old one is kept.".into())
            });
            return;
        }
        if let Some(open) = self.wf.open.borrow_mut().as_mut() {
            open.name = name.trim().to_string();
        }
        if let Some(entry) = self.wf.list.borrow_mut().iter_mut().find(|(i, _)| *i == id) {
            entry.1 = name.trim().to_string();
        }
        let index = self.wf.list.borrow().iter().position(|(i, _)| *i == id);
        if let Some(i) = index {
            self.wf.names.set_row_data(i, name.trim().into());
        }
        self.workflows_schedule_save();
    }

    pub(super) fn workflows_remove_ask(self: &Rc<Self>) {
        let Some((id, name)) = self
            .wf
            .open
            .borrow()
            .as_ref()
            .map(|o| (o.id.clone(), o.name.clone()))
        else {
            return;
        };
        self.ask(
            Question {
                title: "Delete this workflow?".into(),
                body: format!(
                    "\"{name}\" and its nodes, settings and edited scripts are deleted from this PC. Past runs stay in the history. Nothing on your hosts or devices changes."
                ),
                confirm: "Delete workflow".into(),
                danger: true,
            },
            Pending::DeleteWorkflow(id),
        );
    }

    /// The user confirmed: delete the workflow and open the next one.
    pub(crate) fn workflow_delete(self: &Rc<Self>, id: String) {
        self.wf.save_timer.stop();
        if let Err(error) = self.backend.workflow_delete(&id) {
            self.workflows_error(&crate::inventory::friendly(&error));
            return;
        }
        *self.wf.open.borrow_mut() = None;
        self.workflows_reload(None);
    }

    /// Changes the permissions of the open workflow.
    pub(super) fn workflows_set_permission(self: &Rc<Self>, which: &str, on: bool) {
        self.workflows_edit(|wf| {
            let p = &mut wf.permissions;
            match which {
                "root" => p.root = on,
                "shell" => p.shell = on,
                "install" => p.install = on,
                "network" => p.network = on,
                _ => {}
            }
        });
        self.workflows_refresh(true);
    }

    /// Applies `change` to the open graph and saves it soon.
    pub(super) fn workflows_edit(self: &Rc<Self>, change: impl FnOnce(&mut Workflow)) {
        if let Some(open) = self.wf.open.borrow_mut().as_mut() {
            change(&mut open.graph);
        } else {
            return;
        }
        self.workflows_schedule_save();
    }

    pub(super) fn workflows_schedule_save(self: &Rc<Self>) {
        self.with_view(|w| w.set_save_state("Saving...".into()));
        let app = Rc::downgrade(self);
        self.wf
            .save_timer
            .start(slint::TimerMode::SingleShot, SAVE_DELAY, move || {
                if let Some(app) = app.upgrade() {
                    app.workflows_save_now();
                }
            });
    }

    /// Writes a pending change at once (before switching workflows).
    fn workflows_flush(self: &Rc<Self>) {
        if self.wf.save_timer.running() {
            self.wf.save_timer.stop();
            self.workflows_save_now();
        }
    }

    fn workflows_save_now(self: &Rc<Self>) {
        let Some((id, name, mut graph)) = self
            .wf
            .open
            .borrow()
            .as_ref()
            .map(|o| (o.id.clone(), o.name.clone(), o.graph.clone()))
        else {
            return;
        };
        let v = self.wf.view.get();
        graph.viewport = Viewport {
            x: v.pan_x,
            y: v.pan_y,
            zoom: v.zoom,
        };
        let backend = self.backend.clone();
        self.run(
            async move { backend.workflow_save(Some(id), name, &graph) },
            |app, result| {
                let text = match result {
                    Ok(_) => "Saved on this PC.".to_string(),
                    Err(error) => format!("Not saved: {}", crate::inventory::friendly(&error)),
                };
                app.with_view(|w| w.set_save_state(text.into()));
            },
        );
    }

    pub(super) fn workflows_error(&self, text: &str) {
        self.with_view(|w| w.set_error(text.into()));
    }
}
