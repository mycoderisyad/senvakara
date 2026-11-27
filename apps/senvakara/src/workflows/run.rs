//! Running the open workflow: the sudo password for this run (memory only, never saved), the
//! confirmation that names every machine and step, then a run tab like a recipe's.

use std::rc::Rc;

use senvakara_app::workflow::{plan, Binding, Plan};
use senvakara_app::{Secret, WorkflowRunRequest};
use slint::{Model, SharedString};

use super::view::title_of;
use crate::inventory::fact;
use crate::run::{Job, RunPane};
use crate::{App, KeyValue, Pane, Pending, Question, TabInfo};

/// Steps listed one by one in the confirmation; more are counted.
const LISTED_STEPS: usize = 12;

impl App {
    /// The sudo password typed for the next run.
    pub(super) fn workflows_sudo(self: &Rc<Self>, text: &str) {
        *self.wf.sudo.borrow_mut() = (!text.is_empty()).then(|| Secret::new(text.to_string()));
        self.workflows_refresh(false);
    }

    /// The plan of the open graph, when it has no problems.
    pub(super) fn workflows_plan(&self) -> Option<Plan> {
        let open = self.wf.open.borrow();
        plan(&open.as_ref()?.graph, senvakara_app::catalog()).ok()
    }

    /// Why Run cannot be pressed, or "" when it can.
    pub(super) fn workflows_run_reason(&self, problems: usize) -> String {
        if problems > 0 {
            return format!(
                "Fix {} before a run (listed in the inspector).",
                if problems == 1 {
                    "1 problem".to_string()
                } else {
                    format!("{problems} problems")
                }
            );
        }
        let Some(plan) = self.workflows_plan() else {
            return String::new();
        };
        if plan.steps.is_empty() {
            return "Add a step that runs on a host or device.".into();
        }
        if plan.needs_root() && self.wf.sudo.borrow().is_none() {
            return "Enter the sudo password in the workflow panel: some steps run as root.".into();
        }
        String::new()
    }

    fn machine_name(&self, binding: &Binding) -> String {
        self.wf
            .machines
            .borrow()
            .of(binding)
            .map(|m| m.name)
            .unwrap_or_else(|| "a machine no longer in the inventory".into())
    }

    /// Run pressed: save, then ask with every machine and step named.
    pub(super) fn workflows_run_ask(self: &Rc<Self>) {
        let problems = self.wf.problems.borrow().len();
        if !self.workflows_run_reason(problems).is_empty() {
            return;
        }
        let Some(plan) = self.workflows_plan() else {
            return;
        };
        let (name, graph) = match self.wf.open.borrow().as_ref() {
            Some(o) => (o.name.clone(), o.graph.clone()),
            None => return,
        };
        let catalog = senvakara_app::catalog();
        let mut machines: Vec<String> = Vec::new();
        for step in &plan.steps {
            let m = self.machine_name(&step.target);
            if !machines.contains(&m) {
                machines.push(m);
            }
        }
        let mut facts: Vec<KeyValue> = vec![fact("Runs on", machines.join(", "), false)];
        for (i, step) in plan.steps.iter().take(LISTED_STEPS).enumerate() {
            let title = graph
                .node(&step.id)
                .map(|n| title_of(n, catalog))
                .unwrap_or_default();
            let root = if step.needs_root { ", as root" } else { "" };
            facts.push(fact(
                &format!("Step {}", i + 1),
                format!("{title} on {}{root}", self.machine_name(&step.target)),
                false,
            ));
        }
        if plan.steps.len() > LISTED_STEPS {
            facts.push(fact(
                "More",
                format!("{} more steps", plan.steps.len() - LISTED_STEPS),
                false,
            ));
        }
        // Guarded steps (SSH, firewall) and a moving SSH port are named before the run (plan
        // WF §4): confirming the run is the confirmation that Hosts follows the new port.
        for step in &plan.steps {
            let Some(guard) = catalog
                .get(&step.module)
                .and_then(|m| m.manifest.guard.as_ref())
            else {
                continue;
            };
            let title = graph
                .node(&step.id)
                .map(|n| title_of(n, catalog))
                .unwrap_or_default();
            let machine = self.machine_name(&step.target);
            facts.push(fact(
                "Safety net",
                format!(
                    "{title}: {machine} undoes it by itself after {} s unless Senvakara signs in again",
                    guard.rollback_secs
                ),
                false,
            ));
            let new_port = guard
                .port_param
                .as_ref()
                .and_then(|p| step.params.get(p))
                .and_then(|v| match v {
                    senvakara_app::workflow::ParamValue::Int(n) => u16::try_from(*n).ok(),
                    _ => None,
                });
            let current = match &step.target {
                Binding::Host { host_id } => {
                    self.inv.host(host_id).and_then(|h| h.ssh).map(|s| s.port)
                }
                _ => None,
            };
            if let (Some(port), Some(current)) = (new_port, current) {
                if port != current {
                    facts.push(fact(
                        "SSH port",
                        format!("{machine} moves from port {current} to {port}; Hosts is updated once Senvakara signed in on {port}"),
                        false,
                    ));
                }
            }
        }
        let root = plan.steps.iter().filter(|s| s.needs_root).count();
        let warning = if root > 0 {
            format!("{root} step(s) run as root through sudo. Each script is shown in its node before you run it.")
        } else {
            String::new()
        };
        self.ask_with(
            Question {
                title: format!("Run {name}?"),
                body: "Your changes are saved first. Steps run one at a time in order; a failed step skips the steps that wait for it, and Stop ends the running script and everything it started.".into(),
                confirm: "Run workflow".into(),
                danger: false,
            },
            facts,
            &warning,
            Pending::RunWorkflow,
        );
    }

    /// The user confirmed: save, then open a run tab. The sudo password is handed to the run
    /// and forgotten here.
    pub(crate) fn workflows_start(self: &Rc<Self>) {
        let Some(plan) = self.workflows_plan() else {
            return;
        };
        let (id, name, graph) = match self.wf.open.borrow().as_ref() {
            Some(o) => (o.id.clone(), o.name.clone(), o.graph.clone()),
            None => return,
        };
        // The backend reads the workflow from the database: write it now.
        self.wf.save_timer.stop();
        if let Err(error) = self
            .backend
            .workflow_save(Some(id.clone()), name.clone(), &graph)
        {
            self.workflows_error(&crate::inventory::friendly(&error));
            return;
        }
        let mut serials = std::collections::HashMap::new();
        for step in &plan.steps {
            if let Binding::Device { route_id } = &step.target {
                if let Some(device) = self.inv.device(route_id) {
                    serials.insert(route_id.clone(), device.record.serial.clone());
                }
            }
        }
        let catalog = senvakara_app::catalog();
        let labels: Vec<String> = plan
            .steps
            .iter()
            .map(|s| {
                graph
                    .node(&s.id)
                    .map(|n| title_of(n, catalog))
                    .unwrap_or_default()
            })
            .collect();
        let mut machines: Vec<String> = Vec::new();
        for step in &plan.steps {
            let m = self.machine_name(&step.target);
            if !machines.contains(&m) {
                machines.push(m);
            }
        }
        let hue = plan
            .steps
            .first()
            .and_then(|s| self.wf.machines.borrow().of(&s.target))
            .map_or(0, |m| m.hue);
        let request = WorkflowRunRequest {
            workflow_id: id,
            sudo: self.wf.sudo.borrow_mut().take(),
            serials,
        };
        let tab = self.take_id();
        let title = format!("Run {name}");
        let pane = RunPane::start(
            tab,
            self.backend.clone(),
            title.clone(),
            machines.join(", "),
            hue,
            labels,
            Job::Workflow(request),
            crate::notifier(tab),
        );
        self.sessions.borrow_mut().push(Pane::Run(pane));
        self.tabs.push(TabInfo {
            id: tab,
            kind: "run".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.with_view(|v| v.set_sudo_text("".into()));
        self.workflows_refresh(false);
        self.select(self.tabs.row_count() as i32 - 1);
    }
}
