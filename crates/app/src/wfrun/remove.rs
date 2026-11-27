//! Remove (ADR-033): the user asked, and confirmed, that one installed node
//! be taken off its host. Its `remove.sh` runs as a one-step run; data the
//! module created is left on the host by the script's contract.

use super::host::check_os;
use super::remote::{
    check_sudo, err, prepare, run_script, script_text, upload_module_files, Hosts,
};
use super::{secrets, Outcome, RunCtx};
use crate::state::CmdResult;
use crate::workflows::catalog;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{
    preamble, session_exports, without_secrets, Binding, CheckedNode, Executor, Plan,
};
use std::collections::BTreeMap;
use std::time::Duration;

/// Runs the node's `remove.sh` after the user confirmed it, then reports
/// what health says is left.
pub(super) async fn remove_node(
    ctx: &RunCtx,
    hosts: &mut Hosts,
    node: &CheckedNode,
) -> CmdResult<Outcome> {
    let Binding::Host { host_id } = &node.target else {
        return Err(err(
            ErrorCode::InvalidInput,
            "this step runs on an SSH host",
        ));
    };
    let module = catalog()
        .get(&node.module)
        .ok_or_else(|| err(ErrorCode::InvalidInput, "this step's module is unknown"))?;
    let body = script_text(&node.id, ctx, &node.module, "remove.sh")
        .ok_or_else(|| err(ErrorCode::InvalidInput, "this node has nothing to remove"))?;
    let secrets = secrets::resolve(ctx, node).await?;
    let settings = preamble(&without_secrets(&node.params), &BTreeMap::new())
        .map_err(|e| err(ErrorCode::InvalidInput, e))?;
    let timeout = Duration::from_secs(module.manifest.timeout_secs.into());
    let Hosts { by_id, scripts, .. } = hosts;
    let host = prepare(ctx, by_id, host_id).await?;
    check_os(host, module).await?;
    if node.needs_root {
        check_sudo(ctx, host).await?;
    }
    let module_dir = upload_module_files(host, node, module).await?;
    let head = format!(
        "{settings}{}{module_dir}",
        session_exports(&host.session, None)
    );
    let title = ctx.title(&node.id);
    *scripts += 1;
    let n = *scripts;
    let code = run_script(
        ctx,
        host,
        node,
        "remove.sh",
        n,
        &head,
        &body,
        &title,
        timeout,
        &secrets,
    )
    .await?;
    if code != 0 {
        return Err(err(
            ErrorCode::InternalError,
            format!("remove.sh failed with exit status {code}"),
        ));
    }
    let detail = match super::health::health_after(ctx, host, node, &title, 0).await {
        Some((state, _)) => format!("removed; health now: {}", state.label()),
        None => "removed".to_string(),
    };
    Ok(Outcome {
        detail,
        changed: Some(true),
        exit_code: Some(0),
    })
}

/// The run plan of a Remove: only `node_id`, which must be a host action
/// whose module ships `remove.sh`.
pub(super) fn plan_for(plan: Plan, node_id: &str) -> CmdResult<Plan> {
    let invalid = |m: &str| CoreError::new(ErrorCode::InvalidInput, m);
    let step = plan
        .steps
        .into_iter()
        .find(|s| s.id == node_id)
        .ok_or_else(|| invalid("this node is not an action of the workflow"))?;
    let module = catalog()
        .get(&step.module)
        .ok_or_else(|| invalid("this step's module is unknown"))?;
    if module.manifest.executor != Executor::SshScript || !module.files.contains_key("remove.sh") {
        return Err(invalid("this node has nothing to remove"));
    }
    Ok(Plan {
        steps: vec![step],
        waits_for: vec![Vec::new()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use senvakara_workflow::{Binding, CheckedNode};
    use std::collections::BTreeMap;

    fn step(id: &str, module: &str) -> CheckedNode {
        CheckedNode {
            id: id.into(),
            module: module.into(),
            target: Binding::Host {
                host_id: "h".into(),
            },
            params: BTreeMap::new(),
            needs_root: true,
        }
    }

    #[test]
    fn only_a_node_with_a_remove_script_can_be_removed() {
        let plan = Plan {
            steps: vec![step("a", "security.fail2ban"), step("b", "app.adminer")],
            waits_for: vec![Vec::new(), vec![0]],
        };
        let one = plan_for(plan.clone(), "b").unwrap();
        assert_eq!(one.steps.len(), 1);
        assert_eq!(one.steps[0].id, "b");
        assert_eq!(one.waits_for, vec![Vec::<usize>::new()]);
        assert!(
            plan_for(plan.clone(), "a").is_err(),
            "fail2ban has no remove.sh"
        );
        assert!(plan_for(plan, "missing").is_err());
    }
}
