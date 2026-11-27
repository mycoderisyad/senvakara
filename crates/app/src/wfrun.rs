//! Running a workflow (plan WF, ADR-030): validate and order the graph, then
//! carry out each action on its machine. Host actions run bundled or user
//! scripts over SSH (`remote`), Android actions reuse the recipe steps
//! (`device`), Wait is done here. A failed action skips what waits for it;
//! unrelated branches go on. Every step is recorded in `workflow_steps`.
//!
//! Actions run one at a time in plan order. Running different machines side
//! by side is a later step; the order is already valid for it.

mod device;
mod guard;
mod host;
mod log;
mod remote;

use crate::flow::FlowEvent;
use crate::logging;
use crate::sink::Sink;
use crate::state::{db_err, CmdResult};
use crate::workflows::catalog;
use crate::App;
use senvakara_adapters_ssh::Secret;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{plan, Binding, CheckedNode, Executor, Workflow};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// What the UI confirmed before a run.
pub struct WorkflowRunRequest {
    pub workflow_id: String,
    /// Sudo password for steps that run as root; asked per run, never stored
    /// here.
    pub sudo: Option<Secret>,
    /// Serial the user saw for each device route in the workflow (ADR-018).
    pub serials: HashMap<String, String>,
}

/// What a finished action reports.
pub(crate) struct Outcome {
    pub detail: String,
    /// `Some(false)`: it was already as wanted and nothing ran.
    pub changed: Option<bool>,
    pub exit_code: Option<i64>,
}

/// Everything one run shares between its steps.
pub(crate) struct RunCtx {
    pub app: Arc<App>,
    pub run_id: String,
    pub sink: Sink<FlowEvent>,
    pub cancel: Arc<AtomicBool>,
    /// Turns true once `cancel` is set, for `exec_stream`.
    pub cancel_rx: watch::Receiver<bool>,
    pub sudo: Option<Secret>,
    pub serials: HashMap<String, String>,
    pub graph: Workflow,
}

impl RunCtx {
    pub fn log(&self, text: impl Into<String>) {
        (self.sink)(FlowEvent::Log { text: text.into() });
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn title(&self, node_id: &str) -> String {
        self.graph
            .node(node_id)
            .map(|n| {
                if n.title.is_empty() {
                    catalog()
                        .get(&n.module)
                        .map(|m| m.manifest.name.clone())
                        .unwrap_or_else(|| n.module.clone())
                } else {
                    n.title.clone()
                }
            })
            .unwrap_or_else(|| node_id.to_string())
    }
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidInput, message)
}

/// Checks and starts a run; returns its id. Steps, log lines and the final
/// [`FlowEvent::Done`] go to `sink`.
pub(crate) async fn workflow_run(
    app: &Arc<App>,
    req: WorkflowRunRequest,
    sink: Sink<FlowEvent>,
) -> CmdResult<String> {
    let record = app
        .state
        .db
        .lock()
        .unwrap()
        .get_workflow(&req.workflow_id)
        .map_err(|e| db_err("workflow", e))?;
    let graph: Workflow = serde_json::from_value(record.graph)
        .map_err(|e| invalid(format!("this workflow could not be read: {e}")))?;
    let plan = plan(&graph, catalog()).map_err(|problems| {
        let first = problems
            .first()
            .map(|p| p.message.clone())
            .unwrap_or_default();
        invalid(format!(
            "{} problem(s) to fix first, starting with: {first}",
            problems.len()
        ))
    })?;
    if plan.steps.is_empty() {
        return Err(invalid("this workflow has no actions to run"));
    }
    if plan.needs_root() && req.sudo.as_ref().is_none_or(|s| s.is_empty()) {
        return Err(CoreError::new(
            ErrorCode::Unauthorized,
            "some steps run as root: enter the sudo password",
        ));
    }
    for step in &plan.steps {
        if let Binding::Device { route_id } = &step.target {
            if !req.serials.contains_key(route_id) {
                return Err(CoreError::new(
                    ErrorCode::RouteStale,
                    "a phone in this workflow was not confirmed for this run",
                ));
            }
        }
    }

    let names: Vec<(String, String)> = plan
        .steps
        .iter()
        .map(|s| (s.id.clone(), s.module.clone()))
        .collect();
    let refs: Vec<(&str, &str)> = names
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let summary = serde_json::json!({ "workflow": record.name, "steps": plan.steps.len() });
    let run_id = app
        .state
        .db
        .lock()
        .unwrap()
        .create_workflow_run(&record.id, &summary, &refs)
        .map_err(|e| db_err("run", e))?;
    let cancel = Arc::new(AtomicBool::new(false));
    app.flows.insert(&run_id, cancel.clone());
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let ctx = RunCtx {
        app: app.clone(),
        run_id: run_id.clone(),
        sink,
        cancel: cancel.clone(),
        cancel_rx,
        sudo: req.sudo,
        serials: req.serials,
        graph,
    };
    logging::info(&format!("workflow {} run {run_id} started", record.id));

    // `flow_cancel` sets an AtomicBool; remote execs wait on a watch channel.
    let done = Arc::new(AtomicBool::new(false));
    app.rt.spawn({
        let (cancel, done) = (cancel.clone(), done.clone());
        async move {
            while !done.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    cancel_tx.send(true).ok();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    });

    app.rt.spawn(async move {
        let started = Instant::now();
        let (status, message) = execute(&ctx, &plan).await;
        done.store(true, Ordering::Relaxed);
        let mut s = summary;
        s["result"] = serde_json::json!(message);
        s["seconds"] = serde_json::json!(started.elapsed().as_secs());
        let _ = ctx
            .app
            .state
            .db
            .lock()
            .unwrap()
            .finish_run(&ctx.run_id, status, &s);
        (ctx.sink)(FlowEvent::Done {
            status: status.into(),
            message,
        });
        ctx.app.flows.remove(&ctx.run_id);
        logging::info(&format!("workflow run {} finished: {status}", ctx.run_id));
    });
    Ok(run_id)
}

fn step(ctx: &RunCtx, ordinal: usize, node: &CheckedNode, status: &str, detail: Option<String>) {
    let result = detail.as_ref().map(|d| serde_json::json!({ "detail": d }));
    let _ =
        ctx.app
            .state
            .db
            .lock()
            .unwrap()
            .set_step(&ctx.run_id, ordinal, status, result.as_ref());
    (ctx.sink)(FlowEvent::Step {
        ordinal,
        name: ctx.title(&node.id),
        status: status.into(),
        detail,
    });
}

/// Runs the plan; returns the run's final status and a sentence about it.
async fn execute(ctx: &RunCtx, plan: &senvakara_workflow::Plan) -> (&'static str, String) {
    let mut skip: BTreeSet<usize> = BTreeSet::new();
    let mut hosts = remote::Hosts::default();
    let (mut done, mut failed, mut unchanged) = (0, 0, 0);
    for (i, node) in plan.steps.iter().enumerate() {
        if ctx.cancelled() {
            step(ctx, i, node, "cancelled", None);
            continue;
        }
        if skip.contains(&i) {
            step(
                ctx,
                i,
                node,
                "skipped",
                Some("an earlier step it waits for failed".into()),
            );
            continue;
        }
        step(ctx, i, node, "running", None);
        let module = catalog().get(&node.module);
        let executor = module.map(|m| m.manifest.executor);
        let result = match executor {
            Some(Executor::SshScript) => remote::run_node(ctx, &mut hosts, node).await,
            Some(Executor::Adb) | Some(Executor::LocalFlutter) => device::run_node(ctx, node).await,
            Some(Executor::Builtin) => wait(ctx, node).await,
            _ => Err(invalid(format!("{} cannot run", node.module))),
        };
        match result {
            Ok(outcome) => {
                let _ = ctx.app.state.db.lock().unwrap().set_step_outcome(
                    &ctx.run_id,
                    i,
                    outcome.exit_code,
                    outcome.changed,
                );
                done += 1;
                if outcome.changed == Some(false) {
                    unchanged += 1;
                }
                step(ctx, i, node, "succeeded", Some(outcome.detail));
            }
            Err(e) if e.code == ErrorCode::Cancelled => {
                step(ctx, i, node, "cancelled", Some(e.message.clone()));
            }
            Err(e) => {
                failed += 1;
                step(ctx, i, node, "failed", Some(e.message.clone()));
                skip.extend(plan.downstream(i));
            }
        }
    }
    hosts.clean_up(ctx).await;
    if ctx.cancelled() {
        (
            "cancelled",
            "Steps that had not started were not run.".into(),
        )
    } else if failed > 0 {
        ("failed", format!("{failed} failed, {done} done"))
    } else {
        let steps = if done == 1 {
            "1 step done".to_string()
        } else {
            format!("{done} steps done")
        };
        let message = match unchanged {
            0 => steps,
            n => format!("{steps}, {n} already up to date"),
        };
        ("succeeded", message)
    }
}

/// The Wait node: sleeps, but a cancel ends it at once.
async fn wait(ctx: &RunCtx, node: &CheckedNode) -> CmdResult<Outcome> {
    let secs = match node.params.get("seconds") {
        Some(senvakara_workflow::ParamValue::Int(n)) => *n as u64,
        _ => 1,
    };
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if ctx.cancelled() {
            return Err(CoreError::new(
                ErrorCode::Cancelled,
                "cancelled during wait",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(Outcome {
        detail: format!("{secs} s"),
        changed: None,
        exit_code: None,
    })
}
