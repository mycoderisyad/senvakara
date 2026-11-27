//! Turns an automation recipe (FLOW-04, schema v4) into a workflow, so the
//! Workflows canvas can replace Automations without losing saved recipes.
//! A recipe is a straight line of steps on one Android device.

use crate::graph::{Binding, Edge, EdgeKind, Node, Permissions, Workflow};
use std::collections::BTreeMap;

/// One recipe step as stored: a kind from the old allowlist and its
/// parameters.
#[derive(Debug, Clone)]
pub struct RecipeStepIn<'a> {
    pub kind: &'a str,
    pub params: &'a serde_json::Value,
}

/// Old step kind → module id. `flutter_deploy` moved to Deployment, `wait`
/// to Automation; the rest are Android actions.
fn module_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "install_apk" => "android.install_apk",
        "uninstall_package" => "android.uninstall_package",
        "launch_app" => "android.launch_app",
        "device_shell" => "android.device_shell",
        "screenshot" => "android.screenshot",
        "flutter_deploy" => "deployment.flutter_deploy",
        "wait" => "automation.wait",
        _ => return None,
    })
}

const COLUMN: f32 = 260.0;
const DEVICE_ROW: f32 = 220.0;

/// Builds the workflow for one recipe. A recipe without a device keeps its
/// steps but gets a device node with no route, which the canvas asks the
/// user to choose. Unknown step kinds are refused.
pub fn from_recipe(
    route_id: Option<&str>,
    steps: &[RecipeStepIn<'_>],
    install: bool,
    shell: bool,
) -> Result<Workflow, String> {
    let mut wf = Workflow {
        permissions: Permissions {
            install,
            shell,
            ..Permissions::default()
        },
        ..Workflow::default()
    };
    wf.nodes.push(Node {
        id: "device".into(),
        module: "android.device".into(),
        title: String::new(),
        x: 0.0,
        y: DEVICE_ROW,
        params: BTreeMap::new(),
        binding: route_id.map(|r| Binding::Device {
            route_id: r.to_string(),
        }),
        scripts: BTreeMap::new(),
    });
    for (i, step) in steps.iter().enumerate() {
        let module =
            module_for(step.kind).ok_or_else(|| format!("unknown step kind {}", step.kind))?;
        let params = step
            .params
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let id = format!("step-{}", i + 1);
        wf.nodes.push(Node {
            id: id.clone(),
            module: module.into(),
            title: String::new(),
            x: i as f32 * COLUMN,
            y: 0.0,
            params,
            binding: None,
            scripts: BTreeMap::new(),
        });
        wf.edges.push(Edge {
            from: id.clone(),
            to: "device".into(),
            kind: EdgeKind::Target,
        });
        if i > 0 {
            wf.edges.push(Edge {
                from: format!("step-{i}"),
                to: id,
                kind: EdgeKind::After,
            });
        }
    }
    Ok(wf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{plan, Catalog};
    use serde_json::json;

    #[test]
    fn a_recipe_becomes_a_line_of_actions_on_its_device() {
        let p1 = json!({"path": "C:\\b\\app.apk"});
        let p2 = json!({"seconds": 2});
        let p3 = json!({"package": "dev.x.app"});
        let steps = [
            RecipeStepIn {
                kind: "install_apk",
                params: &p1,
            },
            RecipeStepIn {
                kind: "wait",
                params: &p2,
            },
            RecipeStepIn {
                kind: "launch_app",
                params: &p3,
            },
        ];
        let wf = from_recipe(Some("route-1"), &steps, true, false).unwrap();
        let plan = plan(&wf, &Catalog::bundled().unwrap()).unwrap();
        let order: Vec<&str> = plan.steps.iter().map(|s| s.module.as_str()).collect();
        assert_eq!(
            order,
            [
                "android.install_apk",
                "automation.wait",
                "android.launch_app"
            ]
        );
        let device = Binding::Device {
            route_id: "route-1".into(),
        };
        assert!(plan.steps.iter().all(|s| s.target == device));
        assert_eq!(plan.downstream(0), [1, 2].into());
    }

    #[test]
    fn recipe_permissions_carry_over_and_still_gate_the_run() {
        let p = json!({"command": "id"});
        let steps = [RecipeStepIn {
            kind: "device_shell",
            params: &p,
        }];
        let catalog = Catalog::bundled().unwrap();
        let denied = from_recipe(Some("r"), &steps, false, false).unwrap();
        let problems = plan(&denied, &catalog).unwrap_err();
        assert!(
            problems[0].message.contains("run shell commands"),
            "{problems:?}"
        );
        let allowed = from_recipe(Some("r"), &steps, false, true).unwrap();
        assert!(plan(&allowed, &catalog).is_ok());
    }

    #[test]
    fn a_recipe_without_a_device_asks_for_one_and_unknown_kinds_fail() {
        let p = json!({"seconds": 2});
        let wf = from_recipe(
            None,
            &[RecipeStepIn {
                kind: "wait",
                params: &p,
            }],
            false,
            false,
        )
        .unwrap();
        let problems = plan(&wf, &Catalog::bundled().unwrap()).unwrap_err();
        assert!(problems
            .iter()
            .any(|p| p.node.as_deref() == Some("device") && p.message.contains("Android device")));
        assert!(from_recipe(
            None,
            &[RecipeStepIn {
                kind: "teleport",
                params: &p
            }],
            false,
            false
        )
        .is_err());
    }
}
