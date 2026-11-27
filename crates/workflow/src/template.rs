//! Ready-made workflows the user adds from the toolbar. A template only adds
//! nodes and connections; settings it cannot know (user name, key) stay
//! empty, so the canvas asks for them, and it grants no permission: the user
//! turns those on in the workflow settings (SECURITY §6).

use crate::graph::{Binding, Edge, EdgeKind, Node, Workflow};
use std::collections::BTreeSet;

/// "New VPS baseline", in run order (plan WF §7).
pub const VPS_BASELINE: [&str; 6] = [
    "security.system_update",
    "security.sudo_user",
    "security.ssh_hardening",
    "networking.firewall",
    "security.fail2ban",
    "security.unattended_upgrades",
];

/// Horizontal distance between template steps, in world units.
const STEP_X: f32 = 260.0;
/// How far below the host the steps go.
const STEP_Y: f32 = 150.0;

fn fresh_id(used: &mut BTreeSet<String>) -> String {
    let id = (1..)
        .map(|i| format!("n{i}"))
        .find(|id| !used.contains(id))
        .expect("an unused id");
    used.insert(id.clone());
    id
}

fn node(id: String, module: &str, x: f32, y: f32, binding: Option<Binding>) -> Node {
    Node {
        id,
        module: module.to_string(),
        title: String::new(),
        x: x.round(),
        y: y.round(),
        params: Default::default(),
        binding,
        scripts: Default::default(),
    }
}

/// Adds the baseline steps, each running after the one before and all on
/// `host` (an SSH host node already on the canvas), or on a new SSH host
/// node at (`x`, `y`) when `host` is `None`. Returns the added ids, the new
/// host first.
///
/// Fails when `host` is not an SSH host node of `wf`.
pub fn add_vps_baseline(
    wf: &mut Workflow,
    host: Option<&str>,
    x: f32,
    y: f32,
) -> Result<Vec<String>, String> {
    let mut used: BTreeSet<String> = wf.nodes.iter().map(|n| n.id.clone()).collect();
    let mut added = Vec::new();
    let (host_id, hx, hy) = match host {
        Some(id) => {
            let n = wf
                .node(id)
                .filter(|n| n.module == "host.ssh")
                .ok_or_else(|| "choose an SSH host node for the baseline".to_string())?;
            (n.id.clone(), n.x, n.y)
        }
        None => {
            let id = fresh_id(&mut used);
            wf.nodes.push(node(id.clone(), "host.ssh", x, y, None));
            added.push(id.clone());
            (id, x, y)
        }
    };
    let mut previous: Option<String> = None;
    for (i, module) in VPS_BASELINE.iter().enumerate() {
        let id = fresh_id(&mut used);
        let sx = hx + i as f32 * STEP_X;
        wf.nodes
            .push(node(id.clone(), module, sx, hy + STEP_Y, None));
        wf.edges.push(Edge {
            from: id.clone(),
            to: host_id.clone(),
            kind: EdgeKind::Target,
        });
        if let Some(prev) = previous.replace(id.clone()) {
            wf.edges.push(Edge {
                from: prev,
                to: id.clone(),
                kind: EdgeKind::After,
            });
        }
        added.push(id);
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Permissions;
    use crate::{plan, Catalog};
    use serde_json::json;

    #[test]
    fn the_baseline_runs_in_order_once_its_settings_and_permissions_are_given() {
        let catalog = Catalog::bundled().unwrap();
        let mut wf = Workflow::default();
        let added = add_vps_baseline(&mut wf, None, 0.0, 0.0).unwrap();
        assert_eq!(added.len(), 7);
        // A fresh template asks for a machine, settings and permissions.
        assert!(crate::validate(&wf, &catalog).is_err());

        let host = &added[0];
        wf.nodes[0].binding = Some(Binding::Host {
            host_id: "host-1".into(),
        });
        let user = added[2].clone();
        let n = wf.nodes.iter_mut().find(|n| n.id == user).unwrap();
        n.params.insert("username".into(), json!("deploy"));
        n.params.insert(
            "public_key".into(),
            json!("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq0dZ1u0Tqk3gJvGQ1m0nI2x8C7w3m1d2Rk0b4y0aBc me@pc"),
        );
        wf.permissions = Permissions {
            root: true,
            shell: false,
            install: true,
            network: true,
        };
        let p = plan(&wf, &catalog).expect("runnable baseline");
        let modules: Vec<&str> = p.steps.iter().map(|s| s.module.as_str()).collect();
        assert_eq!(modules, VPS_BASELINE);
        assert!(p.steps.iter().all(|s| s.target
            == Binding::Host {
                host_id: "host-1".into()
            }));
        assert!(wf.node(host).is_some());
    }

    #[test]
    fn the_baseline_can_join_a_host_already_on_the_canvas() {
        let mut wf = Workflow::default();
        wf.nodes
            .push(node("n1".into(), "host.ssh", 40.0, 80.0, None));
        wf.nodes
            .push(node("n2".into(), "automation.wait", 0.0, 0.0, None));
        let added = add_vps_baseline(&mut wf, Some("n1"), 0.0, 0.0).unwrap();
        assert_eq!(added.len(), 6);
        assert!(!added.contains(&"n1".to_string()) && !added.contains(&"n2".to_string()));
        let first = wf.node(&added[0]).unwrap();
        assert_eq!((first.x, first.y), (40.0, 230.0));
        assert!(add_vps_baseline(&mut wf, Some("n2"), 0.0, 0.0).is_err());
        assert!(add_vps_baseline(&mut wf, Some("missing"), 0.0, 0.0).is_err());
    }
}
