//! Graph rules checked before anything runs and while the user edits:
//! typed connections, one target per action, no cycles, valid settings,
//! granted permissions and bounded scripts.

use crate::graph::{
    Binding, EdgeKind, Node, Workflow, MAX_EDGES, MAX_NODES, MAX_NODE_ID_LEN, MAX_TITLE_LEN,
};
use crate::module::{BindKind, Executor, Module, Permission, MAX_SCRIPT_LEN, SCRIPT_FILES};
use crate::params::{check_params, ParamValue};
use crate::Catalog;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One thing wrong with a workflow, tied to a node when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub node: Option<String>,
    pub message: String,
}

impl Problem {
    fn on(node: &str, message: impl Into<String>) -> Self {
        Self {
            node: Some(node.to_string()),
            message: message.into(),
        }
    }

    fn graph(message: impl Into<String>) -> Self {
        Self {
            node: None,
            message: message.into(),
        }
    }
}

/// An action node that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedNode {
    pub id: String,
    pub module: String,
    /// The machine it runs on, taken from its target anchor.
    pub target: Binding,
    pub params: BTreeMap<String, ParamValue>,
    pub needs_root: bool,
}

/// Checks the whole workflow. Every problem is reported, not only the first,
/// so the canvas can mark each node.
pub fn validate(wf: &Workflow, catalog: &Catalog) -> Result<Vec<CheckedNode>, Vec<Problem>> {
    let mut problems = Vec::new();
    if wf.nodes.len() > MAX_NODES || wf.edges.len() > MAX_EDGES {
        return Err(vec![Problem::graph(format!(
            "a workflow holds at most {MAX_NODES} nodes and {MAX_EDGES} connections"
        ))]);
    }

    let mut modules: HashMap<&str, &Module> = HashMap::new();
    for node in &wf.nodes {
        if node.id.is_empty() || node.id.len() > MAX_NODE_ID_LEN {
            problems.push(Problem::graph("a node has an empty or overlong id"));
            continue;
        }
        if modules.contains_key(node.id.as_str()) {
            problems.push(Problem::on(&node.id, "two nodes share this id"));
            continue;
        }
        if node.title.chars().count() > MAX_TITLE_LEN {
            problems.push(Problem::on(&node.id, "the title is too long"));
        }
        match catalog.get(&node.module) {
            Some(m) => {
                modules.insert(&node.id, m);
            }
            None => problems.push(Problem::on(
                &node.id,
                format!("unknown node type {}", node.module),
            )),
        }
    }

    // Anchors must point at the kind of machine their module stands for.
    for node in &wf.nodes {
        let Some(m) = modules.get(node.id.as_str()) else {
            continue;
        };
        match (m.manifest.binds, &node.binding) {
            (Some(kind), Some(b)) if BindKind::of(b) == kind => {}
            (Some(kind), _) => problems.push(Problem::on(
                &node.id,
                format!("choose {} for this node", kind.label()),
            )),
            (None, Some(_)) => problems.push(Problem::on(
                &node.id,
                "only host and device nodes point at a machine",
            )),
            (None, None) => {}
        }
    }

    let mut targets: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut seen_edges = BTreeSet::new();
    for e in &wf.edges {
        let (Some(from), Some(to)) = (modules.get(e.from.as_str()), modules.get(e.to.as_str()))
        else {
            problems.push(Problem::graph(format!(
                "a connection points at a missing node ({} → {})",
                e.from, e.to
            )));
            continue;
        };
        if e.from == e.to {
            problems.push(Problem::on(&e.from, "a node cannot connect to itself"));
            continue;
        }
        if !seen_edges.insert((e.from.as_str(), e.to.as_str(), e.kind as u8)) {
            continue;
        }
        match e.kind {
            EdgeKind::Target => {
                if from.is_anchor() || !to.is_anchor() {
                    problems.push(Problem::on(
                        &e.from,
                        "a target connection goes from an action to a host or device",
                    ));
                    continue;
                }
                let kind = to.manifest.binds.expect("anchor has binds");
                if !from.manifest.executor.runs_on().contains(&kind) {
                    problems.push(Problem::on(
                        &e.from,
                        format!("{} cannot run on {}", from.manifest.name, kind.label()),
                    ));
                    continue;
                }
                targets.entry(&e.from).or_default().push(&e.to);
            }
            EdgeKind::After | EdgeKind::DependsOn => {
                if from.is_anchor() || to.is_anchor() {
                    problems.push(Problem::on(
                        &e.from,
                        "order connections link two actions, not hosts or devices",
                    ));
                }
            }
        }
    }

    let mut checked = Vec::new();
    for node in &wf.nodes {
        let Some(m) = modules.get(node.id.as_str()) else {
            continue;
        };
        if m.is_anchor() {
            continue;
        }
        let target = match targets.get(node.id.as_str()).map(Vec::as_slice) {
            Some([one]) => wf.node(one).and_then(|n| n.binding.clone()),
            Some(_) => {
                problems.push(Problem::on(
                    &node.id,
                    "connect this node to one host or device only",
                ));
                continue;
            }
            None => {
                problems.push(Problem::on(
                    &node.id,
                    "connect this node to the host or device it runs on",
                ));
                continue;
            }
        };
        let params = match check_params(&m.manifest.params, &node.params) {
            Ok(p) => p,
            Err(message) => {
                problems.push(Problem::on(&node.id, message));
                continue;
            }
        };
        let needs_root = m.manifest.needs.contains(&Permission::Root)
            || m.manifest
                .root_switch
                .as_ref()
                .is_some_and(|s| params.get(s) == Some(&ParamValue::Bool(true)));
        let mut needs: Vec<Permission> = m.manifest.needs.clone();
        if needs_root && !needs.contains(&Permission::Root) {
            needs.push(Permission::Root);
        }
        for p in needs {
            if !granted(wf, p) {
                problems.push(Problem::on(
                    &node.id,
                    format!(
                        "{} needs the \"{}\" permission for this workflow",
                        m.manifest.name,
                        p.label()
                    ),
                ));
            }
        }
        if let Some(message) = script_problem(node, m) {
            problems.push(Problem::on(&node.id, message));
        }
        if let Some(target) = target {
            checked.push(CheckedNode {
                id: node.id.clone(),
                module: m.manifest.id.clone(),
                target,
                params,
                needs_root,
            });
        }
    }

    if let Err(cycle) = crate::plan::order(wf, &checked) {
        problems.push(Problem::graph(format!(
            "these nodes wait for each other in a loop: {}",
            cycle.join(", ")
        )));
    }
    if problems.is_empty() {
        Ok(checked)
    } else {
        Err(problems)
    }
}

fn granted(wf: &Workflow, p: Permission) -> bool {
    let g = &wf.permissions;
    match p {
        Permission::Root => g.root,
        Permission::Shell => g.shell,
        Permission::Install => g.install,
        Permission::Network => g.network,
    }
}

fn script_problem(node: &Node, m: &Module) -> Option<String> {
    if m.manifest.executor != Executor::SshScript && !node.scripts.is_empty() {
        return Some("this node has no scripts to edit".into());
    }
    for (name, text) in &node.scripts {
        if !SCRIPT_FILES.contains(&name.as_str()) {
            return Some(format!("unexpected script {name}"));
        }
        if !m.manifest.user_scripts && !m.files.contains_key(name) {
            return Some(format!("{} has no {name} to replace", m.manifest.name));
        }
        if text.len() > MAX_SCRIPT_LEN || text.contains('\0') {
            return Some(format!("{name} is larger than 64 KiB"));
        }
    }
    let apply_empty = node
        .scripts
        .get("apply.sh")
        .is_none_or(|s| s.trim().is_empty());
    (m.manifest.user_scripts && apply_empty).then(|| "write the apply script".into())
}

#[cfg(test)]
mod tests;
