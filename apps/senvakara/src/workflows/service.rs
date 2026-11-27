//! Services on the canvas (plan WF §5, ADR-033): the health of nodes whose module ships
//! `health.sh`, shown on their card as a word with its mark and in the inspector's Service
//! section; Remove, which runs the node's `remove.sh` after the user confirmed it (data stays
//! on the host); and secret settings, kept in Credential Manager with only a reference in the
//! graph. Health is checked when a workflow opens (connected hosts not checked yet), after a
//! run and on Refresh; never on a timer.

use std::rc::Rc;
use std::time::Instant;

use senvakara_app::workflow::{plan, Binding, Health, HealthState, Node};
use senvakara_app::{Secret, WorkflowRemoveRequest};
use slint::SharedString;

use super::posture::age_text;
use super::view::{target_of, title_of};
use crate::inventory::fact;
use crate::run::{Job, RunPane};
use crate::{App, Pane, Pending, Question, TabInfo};

/// (tone, word) a card and the Service section show for a health result.
pub fn health_mark(state: HealthState) -> (&'static str, &'static str) {
    let tone = match state {
        HealthState::Running => "live",
        HealthState::Degraded | HealthState::Stopped => "attention",
        HealthState::Unknown => "idle",
    };
    (tone, state.label())
}

/// A health detail as a sentence: capitalised and ending with a full stop.
fn sentence_of(detail: &str) -> String {
    let detail = detail.trim();
    let mut chars = detail.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out: String = first.to_uppercase().chain(chars).collect();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

/// The SSH host a node runs on, when its module reports health.
fn service_host(wf: &senvakara_app::Workflow, node: &Node) -> Option<String> {
    let module = senvakara_app::catalog().get(&node.module)?;
    if !module.has_health() {
        return None;
    }
    match target_of(wf, &node.id)?.binding.as_ref()? {
        Binding::Host { host_id } => Some(host_id.clone()),
        _ => None,
    }
}

impl App {
    /// The health a card shows, when its node was checked.
    pub(super) fn workflows_health_of(&self, node_id: &str) -> Option<Health> {
        self.wf.health.borrow().get(node_id).map(|(h, _)| h.clone())
    }

    /// Checks the connected hosts of the open workflow that run services: those not checked
    /// since the workflow opened, or all of them after a run (`all`). Hosts that are not
    /// connected are left alone: opening a workflow never opens connections.
    pub(super) fn workflows_health_auto(self: &Rc<Self>, all: bool) {
        let pairs: Vec<(String, String)> = match self.wf.open.borrow().as_ref() {
            Some(o) => o
                .graph
                .nodes
                .iter()
                .filter_map(|n| service_host(&o.graph, n).map(|h| (n.id.clone(), h)))
                .collect(),
            None => return,
        };
        let mut hosts: Vec<String> = Vec::new();
        for (node, host) in pairs {
            let connected = self
                .wf
                .machines
                .borrow()
                .hosts
                .get(&host)
                .is_some_and(|m| m.reachable);
            let checked = self.wf.health.borrow().contains_key(&node);
            if connected && (all || !checked) && !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        for host in hosts {
            self.workflows_health_check(host);
        }
    }

    /// Refresh in the Service section: check the selected node's host now (connecting if
    /// needed).
    pub(super) fn workflows_service_refresh(self: &Rc<Self>) {
        let host = {
            let open = self.wf.open.borrow();
            let selected = self.wf.selected.borrow();
            open.as_ref().and_then(|o| {
                o.graph
                    .node(&selected)
                    .and_then(|n| service_host(&o.graph, n))
            })
        };
        if let Some(host) = host {
            self.workflows_health_check(host);
        }
    }

    fn workflows_health_check(self: &Rc<Self>, host: String) {
        let Some((id, name, graph)) = self
            .wf
            .open
            .borrow()
            .as_ref()
            .map(|o| (o.id.clone(), o.name.clone(), o.graph.clone()))
        else {
            return;
        };
        if !self.wf.health_busy.borrow_mut().insert(host.clone()) {
            return;
        }
        // The backend reads the saved workflow: write pending edits first.
        self.wf.save_timer.stop();
        let _ = self.backend.workflow_save(Some(id.clone()), name, &graph);
        self.workflows_fill_inspector(false);
        let backend = self.backend.clone();
        let host_id = host.clone();
        self.run(
            async move { backend.workflow_health(id, host_id).await },
            move |app, result| {
                app.wf.health_busy.borrow_mut().remove(&host);
                let now = Instant::now();
                match result {
                    Ok(list) => {
                        let mut health = app.wf.health.borrow_mut();
                        for (node, h) in list {
                            health.insert(node, (h, now));
                        }
                    }
                    Err(error) => {
                        let text = crate::inventory::friendly(&error);
                        let nodes: Vec<String> = app
                            .wf
                            .open
                            .borrow()
                            .as_ref()
                            .map(|o| {
                                o.graph
                                    .nodes
                                    .iter()
                                    .filter(|n| {
                                        service_host(&o.graph, n).as_deref() == Some(host.as_str())
                                    })
                                    .map(|n| n.id.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut health = app.wf.health.borrow_mut();
                        for node in nodes {
                            let h = Health::unknown(format!("not checked: {text}"));
                            health.insert(node, (h, now));
                        }
                    }
                }
                app.workflows_refresh(false);
            },
        );
    }

    /// Fills the Service section for the selected node (modules with `health.sh`).
    pub(super) fn workflows_fill_service(&self, node: &Node) {
        let catalog = senvakara_app::catalog();
        let Some(module) = catalog.get(&node.module).filter(|m| m.has_health()) else {
            self.with_view(|v| v.set_service_shown(false));
            return;
        };
        let host = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| service_host(&o.graph, node));
        let busy = host
            .as_ref()
            .is_some_and(|h| self.wf.health_busy.borrow().contains(h));
        let cached = self.wf.health.borrow().get(&node.id).cloned();
        let (tone, state, detail) = match (&host, cached) {
            (None, _) => (
                "idle",
                "Not checked yet".to_string(),
                "Connect this node to an SSH host to check it.".to_string(),
            ),
            (Some(_), Some((h, at))) => {
                let (tone, word) = health_mark(h.state);
                let age = age_text(at.elapsed().as_secs() as i64);
                (
                    tone,
                    word.to_string(),
                    format!(
                        "{} Checked {age}, read-only and without sudo.",
                        sentence_of(&h.detail)
                    ),
                )
            }
            (Some(_), None) => (
                "idle",
                "Not checked yet".to_string(),
                "Refresh runs a read-only check as the signed-in user (no sudo, nothing changes)."
                    .to_string(),
            ),
        };
        let removable = module.files.contains_key("remove.sh");
        let (reason, needs_sudo) = self.workflows_remove_reason(node);
        self.with_view(|v| {
            v.set_service_shown(true);
            v.set_service_busy(busy);
            v.set_service_tone(tone.into());
            v.set_service_state(if busy {
                "Checking...".into()
            } else {
                state.into()
            });
            v.set_service_detail(detail.into());
            v.set_service_removable(removable);
            v.set_service_remove_reason(reason.into());
            v.set_service_needs_sudo(removable && needs_sudo);
        });
    }

    /// Why Remove cannot be pressed for `node` ("" when it can), and whether it needs the
    /// sudo password.
    fn workflows_remove_reason(&self, node: &Node) -> (String, bool) {
        let open = self.wf.open.borrow();
        let Some(o) = open.as_ref() else {
            return (String::new(), false);
        };
        let Ok(plan) = plan(&o.graph, senvakara_app::catalog()) else {
            return (
                "Fix the workflow's problems first (listed in the workflow panel).".into(),
                false,
            );
        };
        let Some(step) = plan.steps.iter().find(|s| s.id == node.id) else {
            return ("Connect this node to an SSH host first.".into(), false);
        };
        if step.needs_root && self.wf.sudo.borrow().is_none() {
            return (
                "Enter the sudo password above: removing runs as root.".into(),
                true,
            );
        }
        (String::new(), step.needs_root)
    }

    /// Remove pressed: ask, naming the host, the node and that data stays.
    pub(super) fn workflows_service_remove_ask(self: &Rc<Self>) {
        let Some(id) = Some(self.wf.selected.borrow().clone()).filter(|s| !s.is_empty()) else {
            return;
        };
        let Some((node, graph)) = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(&id).cloned().map(|n| (n, o.graph.clone())))
        else {
            return;
        };
        if !self.workflows_remove_reason(&node).0.is_empty() {
            return;
        }
        let catalog = senvakara_app::catalog();
        let title = title_of(&node, catalog);
        let machine = target_of(&graph, &node.id)
            .and_then(|t| t.binding.as_ref())
            .and_then(|b| self.wf.machines.borrow().of(b))
            .map(|m| m.name)
            .unwrap_or_else(|| "its host".into());
        let facts = vec![
            fact("Host", machine.clone(), false),
            fact("Node", title.clone(), false),
            fact(
                "Data",
                "Kept on the host; the run log says where".to_string(),
                false,
            ),
        ];
        self.ask_with(
            Question {
                title: format!("Remove {title} from {machine}?"),
                body: "Its remove script stops what this node installed and takes it off the host. Data it created stays, so a later install finds it again.".into(),
                confirm: "Remove".into(),
                danger: true,
            },
            facts,
            "The remove script runs as root through sudo. It is shown in the node's Script tab.",
            Pending::RemoveWorkflowNode(id),
        );
    }

    /// The user confirmed a Remove: save, then open a run tab for it.
    pub(crate) fn workflows_remove_start(self: &Rc<Self>, node_id: String) {
        let Some((id, name, graph)) = self
            .wf
            .open
            .borrow()
            .as_ref()
            .map(|o| (o.id.clone(), o.name.clone(), o.graph.clone()))
        else {
            return;
        };
        let Some(node) = graph.node(&node_id).cloned() else {
            return;
        };
        self.wf.save_timer.stop();
        if let Err(error) = self
            .backend
            .workflow_save(Some(id.clone()), name.clone(), &graph)
        {
            self.workflows_error(&crate::inventory::friendly(&error));
            return;
        }
        let title = title_of(&node, senvakara_app::catalog());
        let machine = target_of(&graph, &node_id)
            .and_then(|t| t.binding.as_ref())
            .and_then(|b| self.wf.machines.borrow().of(b));
        let (machine_name, hue) = machine.map_or((String::new(), 0), |m| (m.name, m.hue));
        let request = WorkflowRemoveRequest {
            workflow_id: id,
            node_id: node_id.clone(),
            sudo: self.wf.sudo.borrow_mut().take(),
        };
        // The node's health is stale once it is removed.
        self.wf.health.borrow_mut().remove(&node_id);
        let tab = self.take_id();
        let tab_title = format!("Remove {title}");
        let pane = RunPane::start(
            tab,
            self.backend.clone(),
            tab_title.clone(),
            machine_name,
            hue,
            vec![format!("Remove {title}")],
            Job::WorkflowRemove(request),
            crate::notifier(tab),
        );
        self.sessions.borrow_mut().push(Pane::Run(pane));
        self.tabs.push(TabInfo {
            id: tab,
            kind: "run".into(),
            title: tab_title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.with_view(|v| v.set_sudo_text("".into()));
        self.workflows_refresh(false);
        self.select(slint::Model::row_count(&*self.tabs) as i32 - 1);
    }

    /// Save on a secret setting: the value goes to Credential Manager, the node keeps the
    /// reference, and a reference it replaces is deleted.
    pub(super) fn workflows_secret_save(self: &Rc<Self>, name: &str, value: &str) {
        let Some(id) = Some(self.wf.selected.borrow().clone()).filter(|s| !s.is_empty()) else {
            return;
        };
        if value.is_empty() {
            return;
        }
        let backend = self.backend.clone();
        let secret = Secret::new(value.to_string());
        let name = name.to_string();
        self.run(
            async move { backend.workflow_secret_put(secret).await },
            move |app, result| match result {
                Ok(reference) => {
                    let old = app.workflows_secret_set(&id, &name, Some(reference));
                    app.workflows_secret_forget(old);
                }
                Err(error) => app.workflows_error(&crate::inventory::friendly(&error)),
            },
        );
    }

    /// Clear on a secret setting: the node forgets the reference and the value is deleted.
    pub(super) fn workflows_secret_clear(self: &Rc<Self>, name: &str) {
        let Some(id) = Some(self.wf.selected.borrow().clone()).filter(|s| !s.is_empty()) else {
            return;
        };
        let old = self.workflows_secret_set(&id, name, None);
        self.workflows_secret_forget(old);
    }

    /// Sets (or clears) a secret setting of node `id`; returns the reference it held.
    fn workflows_secret_set(
        self: &Rc<Self>,
        id: &str,
        name: &str,
        reference: Option<String>,
    ) -> Option<String> {
        let old = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(id))
            .and_then(|n| n.params.get(name))
            .and_then(|v| v.as_str())
            .filter(|s| s.starts_with("secret-"))
            .map(str::to_string);
        let value = reference.map_or(serde_json::Value::Null, serde_json::Value::String);
        self.workflows_edit(|wf| super::edit::set_param(wf, id, name, value));
        self.workflows_refresh(true);
        old
    }

    /// Deletes a replaced reference unless another node of the open workflow still uses it.
    fn workflows_secret_forget(self: &Rc<Self>, old: Option<String>) {
        let Some(old) = old else { return };
        let used = self.wf.open.borrow().as_ref().is_some_and(|o| {
            o.graph
                .nodes
                .iter()
                .any(|n| n.params.values().any(|v| v.as_str() == Some(old.as_str())))
        });
        if used {
            return;
        }
        let backend = self.backend.clone();
        self.run(
            async move { backend.workflow_secret_delete(old).await },
            |_, _| {},
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_reads_as_a_word_with_a_mark_never_colour_alone() {
        assert_eq!(health_mark(HealthState::Running), ("live", "Running"));
        assert_eq!(
            health_mark(HealthState::Degraded),
            ("attention", "Degraded")
        );
        assert_eq!(health_mark(HealthState::Stopped), ("attention", "Stopped"));
        assert_eq!(health_mark(HealthState::Unknown), ("idle", "Unknown"));
    }

    #[test]
    fn a_health_detail_reads_as_a_sentence_before_the_check_time() {
        assert_eq!(
            sentence_of("Docker is running but nothing answers on 127.0.0.1:8081"),
            "Docker is running but nothing answers on 127.0.0.1:8081."
        );
        assert_eq!(sentence_of("redis-server failed."), "Redis-server failed.");
        assert_eq!(sentence_of("  "), "");
    }
}
