//! Service health over SSH (plan WF §5, ADR-033): one `bash -s` exec per
//! host runs every `health.sh` of a workflow as the signed-in user, never
//! sudo and never with secrets. On demand only: after an apply, when the
//! user opens a workflow (connected hosts) or presses Refresh.

use super::remote::{script_text, HostRun};
use super::RunCtx;
use crate::state::{db_err, CmdResult};
use crate::workflows::catalog;
use crate::App;
use senvakara_adapters_ssh::{ExecEnd, ExecEvent, ExecLimits, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{
    batch_script, parse_batch, probe_head, probes, CheckedNode, Health, HealthState, Workflow,
    PROBE_SECS,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Output kept from one host's probes; more is not a health report.
const MAX_OUTPUT: usize = 256 * 1024;
/// Longest a whole batch may take.
const MAX_BATCH: Duration = Duration::from_secs(300);

/// Runs `(head, body)` probes on `conn` as the signed-in user and returns
/// one result per probe.
pub(super) async fn run_batch(
    conn: &SshConnection,
    probes: &[(&str, &str)],
    wait_secs: u32,
) -> CmdResult<Vec<Health>> {
    if probes.is_empty() {
        return Ok(Vec::new());
    }
    let script = batch_script(probes, wait_secs);
    let per = u64::from(PROBE_SECS + wait_secs + 3);
    let timeout = Duration::from_secs(per * probes.len() as u64 + 10).min(MAX_BATCH);
    let mut stdout: Vec<u8> = Vec::new();
    let report = conn
        .exec_stream(
            &["bash", "-s"],
            Some(script.as_bytes()),
            ExecLimits {
                timeout,
                drain: Duration::from_secs(2),
            },
            watch::channel(false).1,
            |event| {
                if let ExecEvent::Stdout(bytes) = event {
                    if stdout.len() < MAX_OUTPUT {
                        stdout.extend_from_slice(&bytes);
                    }
                }
            },
            || async {},
        )
        .await
        .map_err(|e| crate::state::to_core("check service health", e))?;
    let text = String::from_utf8_lossy(&stdout);
    let mut results = parse_batch(&text, probes.len());
    if !matches!(report.end, ExecEnd::Exited(_)) {
        // Whatever finished before the cut still counts.
        for h in results.iter_mut() {
            if h.detail == "the host did not run this check" {
                *h = Health::unknown("the health check did not finish");
            }
        }
    }
    Ok(results)
}

/// Checks every service node of `workflow_id` that runs on `host_id`
/// (connecting when needed: the user asked). Returns `(node id, health)`.
pub(crate) async fn workflow_health(
    app: &Arc<App>,
    workflow_id: String,
    host_id: String,
) -> CmdResult<Vec<(String, Health)>> {
    let record = app
        .state
        .db
        .lock()
        .unwrap()
        .get_workflow(&workflow_id)
        .map_err(|e| db_err("workflow", e))?;
    let graph: Workflow = serde_json::from_value(record.graph).map_err(|e| {
        CoreError::new(
            ErrorCode::InvalidInput,
            format!("this workflow could not be read: {e}"),
        )
    })?;
    let (all, skipped) = probes(&graph, catalog());
    let mine: Vec<_> = all.into_iter().filter(|p| p.host_id == host_id).collect();
    let mut out: Vec<(String, Health)> = skipped
        .into_iter()
        .filter(|(id, _)| {
            // Skipped nodes carry no host; keep those on this host only.
            graph
                .edges
                .iter()
                .any(|e| e.from == *id && host_of(&graph, &e.to) == Some(host_id.as_str()))
        })
        .collect();
    if mine.is_empty() {
        return Ok(out);
    }
    let conn = app.state.connection(&host_id).await?;
    let pairs: Vec<(&str, &str)> = mine
        .iter()
        .map(|p| (p.head.as_str(), p.body.as_str()))
        .collect();
    let results = run_batch(&conn, &pairs, 0).await?;
    out.extend(mine.into_iter().map(|p| p.node_id).zip(results));
    Ok(out)
}

/// Runs the node's `health.sh` once (as the signed-in user, no secrets),
/// letting it wait up to `wait_secs` for the service; logs and returns the
/// result. `None` when the module has no health check.
pub(super) async fn health_after(
    ctx: &RunCtx,
    host: &HostRun,
    node: &CheckedNode,
    title: &str,
    wait_secs: u32,
) -> Option<(HealthState, String)> {
    let body = script_text(&node.id, ctx, &node.module, "health.sh")?;
    let head = probe_head(&node.params, wait_secs);
    let result = run_batch(&host.conn, &[(&head, &body)], wait_secs).await;
    let health = match result {
        Ok(mut list) if !list.is_empty() => list.remove(0),
        Ok(_) => return None,
        Err(e) => Health::unknown(format!("not checked: {}", e.message)),
    };
    ctx.log(format!(
        "[{title}] health: {} ({})",
        health.state.label(),
        health.detail
    ));
    Some((health.state, health.detail))
}

fn host_of<'a>(graph: &'a Workflow, node_id: &str) -> Option<&'a str> {
    match graph.node(node_id)?.binding.as_ref()? {
        senvakara_workflow::Binding::Host { host_id } => Some(host_id),
        _ => None,
    }
}
