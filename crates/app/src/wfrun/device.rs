//! Android actions of a workflow: the recipe steps of FLOW-04, bound to the
//! device route the node runs on and to the serial the user confirmed for
//! this run (ADR-018).

use super::{Outcome, RunCtx};
use crate::android::resolve;
use crate::android_tools::confirm_target;
use crate::recipes::{run_step, Ctx};
use crate::state::CmdResult;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::RecipeStep;
use senvakara_workflow::{Binding, CheckedNode, ParamValue};

/// Module id → recipe step kind.
fn step_kind(module: &str) -> Option<&'static str> {
    Some(match module {
        "android.install_apk" => "install_apk",
        "android.uninstall_package" => "uninstall_package",
        "android.launch_app" => "launch_app",
        "android.device_shell" => "device_shell",
        "android.screenshot" => "screenshot",
        "deployment.flutter_deploy" => "flutter_deploy",
        _ => return None,
    })
}

/// The checked settings as the JSON the recipe steps read.
fn params_json(node: &CheckedNode) -> serde_json::Value {
    let map = node
        .params
        .iter()
        .filter_map(|(k, v)| {
            let v = match v {
                ParamValue::Text(s) => serde_json::json!(s),
                ParamValue::Int(n) => serde_json::json!(n),
                ParamValue::Bool(b) => serde_json::json!(b),
                ParamValue::SecretRef(_) => return None,
            };
            Some((k.clone(), v))
        })
        .collect();
    serde_json::Value::Object(map)
}

pub(crate) async fn run_node(ctx: &RunCtx, node: &CheckedNode) -> CmdResult<Outcome> {
    let Binding::Device { route_id } = &node.target else {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "this step runs on an Android device",
        ));
    };
    let kind = step_kind(&node.module).ok_or_else(|| {
        CoreError::new(
            ErrorCode::InvalidInput,
            format!("{} cannot run", node.module),
        )
    })?;
    let expected = ctx.serials.get(route_id).cloned().unwrap_or_default();
    let (record, server, t) = resolve(&ctx.app.state, &ctx.app.android, route_id).await?;
    confirm_target(&record, &expected)?;
    let step_ctx = Ctx {
        app: ctx.app.clone(),
        run_id: ctx.run_id.clone(),
        sink: ctx.sink.clone(),
        cancel: ctx.cancel.clone(),
        server,
        t,
        route_id: route_id.clone(),
        serial: record.serial.clone(),
        device_name: record.display_name.clone(),
    };
    let detail = run_step(
        &step_ctx,
        &RecipeStep {
            kind: kind.into(),
            params: params_json(node),
        },
    )
    .await?;
    Ok(Outcome {
        detail,
        changed: None,
        exit_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_android_module_maps_to_a_recipe_step() {
        let catalog = crate::workflows::catalog();
        for module in [
            "android.install_apk",
            "android.uninstall_package",
            "android.launch_app",
            "android.device_shell",
            "android.screenshot",
            "deployment.flutter_deploy",
        ] {
            assert!(catalog.get(module).is_some(), "{module} not in the catalog");
            assert!(step_kind(module).is_some(), "{module} has no step");
        }
        assert!(step_kind("automation.custom_script").is_none());
    }
}
