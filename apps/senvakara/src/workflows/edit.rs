//! Changes the canvas makes to a workflow graph. Pure functions on `Workflow`, so every rule
//! of how connections are made is unit-tested without a window.

use std::collections::BTreeSet;

use senvakara_app::workflow::{Binding, Edge, EdgeKind, Node};
use senvakara_app::{Catalog, Workflow};

/// A fresh node id: `n1`, `n2`, ... skipping ids in use.
pub fn new_node_id(wf: &Workflow) -> String {
    let used: BTreeSet<&str> = wf.nodes.iter().map(|n| n.id.as_str()).collect();
    (1..)
        .map(|i| format!("n{i}"))
        .find(|id| !used.contains(id.as_str()))
        .expect("an unused id")
}

/// Adds a node of `module` with its top-left corner at world (`x`, `y`); returns its id.
pub fn add_node(wf: &mut Workflow, module: &str, x: f32, y: f32) -> String {
    let id = new_node_id(wf);
    wf.nodes.push(Node {
        id: id.clone(),
        module: module.to_string(),
        title: String::new(),
        x: x.round(),
        y: y.round(),
        params: Default::default(),
        binding: if module == "host.this_pc" {
            Some(Binding::LocalPc)
        } else {
            None
        },
        scripts: Default::default(),
    });
    id
}

pub fn move_node(wf: &mut Workflow, id: &str, x: f32, y: f32) {
    if let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) {
        n.x = x.round();
        n.y = y.round();
    }
}

/// Removes a node and every connection touching it.
pub fn delete_node(wf: &mut Workflow, id: &str) {
    wf.nodes.retain(|n| n.id != id);
    wf.edges.retain(|e| e.from != id && e.to != id);
}

fn is_anchor(wf: &Workflow, catalog: &Catalog, id: &str) -> Option<bool> {
    let node = wf.node(id)?;
    Some(catalog.get(&node.module).is_some_and(|m| m.is_anchor()))
}

/// Whether `to` already runs (directly or not) before `from`, so `from` → `to` would close a
/// loop.
fn reaches(wf: &Workflow, start: &str, goal: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut todo = vec![start.to_string()];
    while let Some(cur) = todo.pop() {
        if cur == goal {
            return true;
        }
        if !seen.insert(cur.clone()) {
            continue;
        }
        for e in wf.edges.iter().filter(|e| e.kind.orders() && e.from == cur) {
            todo.push(e.to.clone());
        }
    }
    false
}

/// Connects two nodes the way the user dragged them, choosing the kind of connection:
/// action → host or device = "runs on" (replacing the action's previous machine; a drag from
/// the machine to the action works too), action → action = "runs after". Returns the
/// connection made, or why none was.
pub fn connect(wf: &mut Workflow, catalog: &Catalog, from: &str, to: &str) -> Result<Edge, String> {
    if from == to {
        return Err("A node cannot connect to itself.".into());
    }
    let (Some(from_anchor), Some(to_anchor)) =
        (is_anchor(wf, catalog, from), is_anchor(wf, catalog, to))
    else {
        return Err("That node no longer exists.".into());
    };
    let edge = match (from_anchor, to_anchor) {
        (true, true) => {
            return Err("Hosts and devices do not connect to each other. Connect an action to the machine it runs on.".into())
        }
        (false, true) | (true, false) => {
            let (action, anchor) = if to_anchor { (from, to) } else { (to, from) };
            wf.edges
                .retain(|e| !(e.kind == EdgeKind::Target && e.from == action));
            Edge {
                from: action.to_string(),
                to: anchor.to_string(),
                kind: EdgeKind::Target,
            }
        }
        (false, false) => {
            if wf
                .edges
                .iter()
                .any(|e| e.kind.orders() && e.from == from && e.to == to)
            {
                return Err("These two actions are already connected.".into());
            }
            if reaches(wf, to, from) {
                return Err("That connection would make the actions wait for each other in a loop.".into());
            }
            Edge {
                from: from.to_string(),
                to: to.to_string(),
                kind: EdgeKind::After,
            }
        }
    };
    wf.edges.push(edge.clone());
    Ok(edge)
}

/// Removes the connection between two nodes, whatever its kind.
pub fn disconnect(wf: &mut Workflow, from: &str, to: &str) {
    wf.edges.retain(|e| !(e.from == from && e.to == to));
}

/// Switches an order connection between "runs after" and "uses the service of".
pub fn set_order_kind(wf: &mut Workflow, from: &str, to: &str, uses_service: bool) {
    for e in wf
        .edges
        .iter_mut()
        .filter(|e| e.kind.orders() && e.from == from && e.to == to)
    {
        e.kind = if uses_service {
            EdgeKind::DependsOn
        } else {
            EdgeKind::After
        };
    }
}

/// Sets a setting from what the user typed; an empty text removes it so the default applies.
/// Checks happen in validation, so a half-typed value is kept as typed.
pub fn set_param(wf: &mut Workflow, id: &str, name: &str, value: serde_json::Value) {
    let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) else {
        return;
    };
    let empty = value.is_null() || value.as_str().is_some_and(|s| s.is_empty());
    if empty {
        n.params.remove(name);
    } else {
        n.params.insert(name.to_string(), value);
    }
}

pub fn set_title(wf: &mut Workflow, id: &str, title: &str) {
    if let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) {
        n.title = title.trim().chars().take(120).collect();
    }
}

pub fn set_binding(wf: &mut Workflow, id: &str, binding: Option<Binding>) {
    if let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) {
        n.binding = binding;
    }
}

/// Stores an edited script; text equal to the bundled file (or empty for a bundled module)
/// removes the override so the node follows catalog updates again.
pub fn set_script(wf: &mut Workflow, catalog: &Catalog, id: &str, file: &str, text: &str) {
    let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) else {
        return;
    };
    let bundled = catalog
        .get(&n.module)
        .and_then(|m| m.files.get(file))
        .map(String::as_str);
    let text = text.replace("\r\n", "\n");
    let user_scripts = catalog
        .get(&n.module)
        .is_some_and(|m| m.manifest.user_scripts);
    if Some(text.as_str()) == bundled || (!user_scripts && text.trim().is_empty()) {
        n.scripts.remove(file);
    } else {
        n.scripts.insert(file.to_string(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::bundled().unwrap()
    }

    fn graph() -> (Workflow, Catalog) {
        let c = catalog();
        let mut wf = Workflow::default();
        add_node(&mut wf, "host.ssh", 0.0, 0.0); // n1
        add_node(&mut wf, "automation.custom_script", 300.0, 0.0); // n2
        add_node(&mut wf, "automation.custom_script", 600.0, 0.0); // n3
        add_node(&mut wf, "host.ssh", 0.0, 200.0); // n4
        (wf, c)
    }

    #[test]
    fn ids_are_short_and_never_reused_while_taken() {
        let (mut wf, _) = graph();
        assert_eq!(new_node_id(&wf), "n5");
        delete_node(&mut wf, "n2");
        assert_eq!(new_node_id(&wf), "n2");
        // This PC needs no choice: it is bound when added.
        let pc = add_node(&mut wf, "host.this_pc", 0.0, 0.0);
        assert_eq!(wf.node(&pc).unwrap().binding, Some(Binding::LocalPc));
    }

    #[test]
    fn dragging_between_an_action_and_a_machine_makes_one_runs_on_link() {
        let (mut wf, c) = graph();
        let e = connect(&mut wf, &c, "n2", "n1").unwrap();
        assert_eq!(
            (e.from.as_str(), e.to.as_str(), e.kind),
            ("n2", "n1", EdgeKind::Target)
        );
        // From the machine to the action works too, and replaces the old machine.
        let e = connect(&mut wf, &c, "n4", "n2").unwrap();
        assert_eq!((e.from.as_str(), e.to.as_str()), ("n2", "n4"));
        let targets: Vec<_> = wf
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Target)
            .collect();
        assert_eq!(targets.len(), 1);
        assert!(connect(&mut wf, &c, "n1", "n4")
            .unwrap_err()
            .contains("do not connect"));
    }

    #[test]
    fn actions_chain_in_order_and_loops_or_repeats_are_refused() {
        let (mut wf, c) = graph();
        assert_eq!(
            connect(&mut wf, &c, "n2", "n3").unwrap().kind,
            EdgeKind::After
        );
        assert!(connect(&mut wf, &c, "n2", "n3")
            .unwrap_err()
            .contains("already"));
        assert!(connect(&mut wf, &c, "n3", "n2")
            .unwrap_err()
            .contains("loop"));
        assert!(connect(&mut wf, &c, "n2", "n2").is_err());
        assert!(connect(&mut wf, &c, "n2", "gone").is_err());
        set_order_kind(&mut wf, "n2", "n3", true);
        assert_eq!(wf.edges[0].kind, EdgeKind::DependsOn);
        // Still an order link: the loop check sees it.
        assert!(connect(&mut wf, &c, "n3", "n2").is_err());
        disconnect(&mut wf, "n2", "n3");
        assert!(wf.edges.is_empty());
    }

    #[test]
    fn deleting_a_node_drops_its_connections() {
        let (mut wf, c) = graph();
        connect(&mut wf, &c, "n2", "n1").unwrap();
        connect(&mut wf, &c, "n2", "n3").unwrap();
        connect(&mut wf, &c, "n3", "n1").unwrap();
        delete_node(&mut wf, "n2");
        assert_eq!(wf.edges.len(), 1);
        assert_eq!(wf.nodes.len(), 3);
    }

    #[test]
    fn empty_settings_fall_back_and_titles_are_trimmed() {
        let (mut wf, _) = graph();
        set_param(&mut wf, "n2", "as_root", serde_json::json!(true));
        assert_eq!(wf.node("n2").unwrap().params.len(), 1);
        set_param(&mut wf, "n2", "as_root", serde_json::json!(""));
        assert!(wf.node("n2").unwrap().params.is_empty());
        set_title(&mut wf, "n2", "  Install tools  ");
        assert_eq!(wf.node("n2").unwrap().title, "Install tools");
    }

    #[test]
    fn a_custom_script_keeps_its_text_and_moves_round_to_whole_units() {
        let (mut wf, c) = graph();
        set_script(&mut wf, &c, "n2", "apply.sh", "apt-get update\r\n");
        assert_eq!(
            wf.node("n2").unwrap().scripts["apply.sh"],
            "apt-get update\n"
        );
        set_script(&mut wf, &c, "n2", "apply.sh", "");
        // A custom script node keeps even an empty script: validation then asks for one.
        assert_eq!(wf.node("n2").unwrap().scripts["apply.sh"], "");
        move_node(&mut wf, "n2", 10.4, 20.6);
        assert_eq!(
            (wf.node("n2").unwrap().x, wf.node("n2").unwrap().y),
            (10.0, 21.0)
        );
    }
}
