//! What the canvas and palette show, built from the graph, the catalog and the inventory:
//! node cards with their honest state, connection curves and palette rows. Pure functions;
//! Slint only draws the result.

use std::collections::HashMap;

use senvakara_app::workflow::{Binding, EdgeKind, Node};
use senvakara_app::{Catalog, Workflow, WorkflowProblem};

use super::geometry::{curve, in_port, node_rect, out_port, View};
use crate::{WfEdge, WfModule, WfNode};

/// A machine as the inventory knows it: name, colour and whether it can be reached now.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    pub name: String,
    pub hue: i32,
    pub reachable: bool,
    /// "Connected" | "Not connected" | "Available" | "Offline" ...
    pub state: String,
}

/// Hosts by host id and devices by route id.
#[derive(Debug, Clone, Default)]
pub struct Machines {
    pub hosts: HashMap<String, Machine>,
    pub devices: HashMap<String, Machine>,
}

impl Machines {
    pub fn of(&self, binding: &Binding) -> Option<Machine> {
        match binding {
            Binding::Host { host_id } => self.hosts.get(host_id).cloned(),
            Binding::Device { route_id } => self.devices.get(route_id).cloned(),
            Binding::LocalPc => Some(Machine {
                name: "This PC".into(),
                hue: 0,
                reachable: true,
                state: "Ready".into(),
            }),
        }
    }
}

/// Problems by node id; the `""` key holds the workflow-wide ones.
pub fn problems_by_node(problems: &[WorkflowProblem]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for p in problems {
        out.entry(p.node.clone().unwrap_or_default())
            .or_default()
            .push(sentence(&p.message));
    }
    out
}

/// Validation speaks in lower case; the inspector shows sentences.
pub fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}

pub fn icon_of(module: &str) -> &'static str {
    match module {
        "host.ssh" => "host",
        "host.this_pc" => "pc",
        "android.device" => "device",
        "automation.custom_script" => "script",
        _ => "action",
    }
}

/// The title a node shows: its own, or its module's name.
pub fn title_of(node: &Node, catalog: &Catalog) -> String {
    if !node.title.is_empty() {
        return node.title.clone();
    }
    catalog
        .get(&node.module)
        .map(|m| m.manifest.name.clone())
        .unwrap_or_else(|| node.module.clone())
}

/// The anchor an action is connected to, if any.
pub fn target_of<'a>(wf: &'a Workflow, id: &str) -> Option<&'a Node> {
    wf.edges
        .iter()
        .find(|e| e.kind == EdgeKind::Target && e.from == id)
        .and_then(|e| wf.node(&e.to))
}

/// One card: title, category, machine and state in words.
pub fn node_row(
    wf: &Workflow,
    node: &Node,
    catalog: &Catalog,
    machines: &Machines,
    problems: &HashMap<String, Vec<String>>,
    view: &View,
    selected: &str,
) -> WfNode {
    let module = catalog.get(&node.module);
    let anchor = module.is_some_and(|m| m.is_anchor());
    let machine = if anchor {
        node.binding.as_ref().and_then(|b| machines.of(b))
    } else {
        target_of(wf, &node.id)
            .and_then(|t| t.binding.as_ref())
            .and_then(|b| machines.of(b))
    };
    let detail = match (&machine, anchor) {
        (Some(m), _) => m.name.clone(),
        (None, true) if node.binding.is_some() => "No longer in the inventory".into(),
        (None, true) => "Not chosen".into(),
        // Connected to a host or device node that has no machine chosen yet.
        (None, false) => match target_of(wf, &node.id) {
            Some(t) => format!("On {}", title_of(t, catalog)),
            None => "Not connected to a machine".into(),
        },
    };
    let has_problem = problems.get(&node.id).is_some_and(|p| !p.is_empty());
    let (tone, state) = if has_problem {
        ("attention", "Needs setup".to_string())
    } else if anchor {
        match &machine {
            Some(m) if m.reachable => ("live", m.state.clone()),
            Some(m) => ("idle", m.state.clone()),
            None => ("attention", "Needs setup".into()),
        }
    } else {
        ("idle", "Not run yet".into())
    };
    let r = node_rect(view, node.x, node.y);
    WfNode {
        id: node.id.clone().into(),
        title: title_of(node, catalog).into(),
        kind: module
            .map(|m| m.manifest.category.label())
            .unwrap_or("Unknown")
            .into(),
        detail: detail.into(),
        icon: icon_of(&node.module).into(),
        hue: if anchor {
            machine.as_ref().map_or(0, |m| m.hue)
        } else {
            0
        },
        anchor,
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
        selected: node.id == selected,
        tone: tone.into(),
        state_text: state.into(),
        custom: module.is_some_and(|m| !m.manifest.user_scripts) && !node.scripts.is_empty(),
    }
}

/// (tone, word) a card shows for its step in the last run of the workflow.
pub fn last_run_state(status: &str, changed: Option<bool>) -> (&'static str, &'static str) {
    match (status, changed) {
        ("succeeded", Some(false)) => ("live", "Up to date"),
        ("succeeded", _) => ("live", "Done in the last run"),
        ("failed", _) => ("attention", "Failed in the last run"),
        ("cancelled", _) => ("idle", "Cancelled in the last run"),
        ("skipped", _) => ("idle", "Skipped in the last run"),
        ("running", _) => ("busy", "Running"),
        _ => ("idle", "Not run yet"),
    }
}

/// One connection curve. "Runs on" links are drawn from the machine to the action, so a
/// machine's actions fan out to its right; order links go from the earlier action.
pub fn edge_row(wf: &Workflow, index: usize, view: &View) -> WfEdge {
    let e = &wf.edges[index];
    let (from, to) = match e.kind {
        EdgeKind::Target => (&e.to, &e.from),
        _ => (&e.from, &e.to),
    };
    let (Some(a), Some(b)) = (wf.node(from), wf.node(to)) else {
        return WfEdge::default();
    };
    let c = curve(
        out_port(&node_rect(view, a.x, a.y)),
        in_port(&node_rect(view, b.x, b.y)),
        view.zoom,
    );
    WfEdge {
        x: c.x,
        y: c.y,
        w: c.w,
        h: c.h,
        commands: c.commands.into(),
        kind: match e.kind {
            EdgeKind::Target => "target",
            EdgeKind::After => "after",
            EdgeKind::DependsOn => "depends_on",
        }
        .into(),
    }
}

/// Palette rows: one heading per category, then its modules, filtered by `query` (name,
/// summary or category, any case), in catalog category order.
pub fn palette_rows(catalog: &Catalog, query: &str) -> Vec<WfModule> {
    let q = query.trim().to_lowercase();
    let mut rows = Vec::new();
    for (category, modules) in catalog.by_category() {
        let label = category.label();
        let hits: Vec<_> = modules
            .into_iter()
            .filter(|m| {
                q.is_empty()
                    || m.manifest.name.to_lowercase().contains(&q)
                    || m.manifest.summary.to_lowercase().contains(&q)
                    || label.to_lowercase().contains(&q)
            })
            .collect();
        if hits.is_empty() {
            continue;
        }
        rows.push(WfModule {
            id: Default::default(),
            name: label.into(),
            summary: Default::default(),
            header: true,
        });
        rows.extend(hits.into_iter().map(|m| WfModule {
            id: m.manifest.id.clone().into(),
            name: m.manifest.name.clone().into(),
            summary: m.manifest.summary.clone().into(),
            header: false,
        }));
    }
    rows
}

/// "100 %" for the zoom button.
pub fn zoom_text(zoom: f32) -> String {
    format!("{} %", (zoom * 100.0).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::edit::{add_node, connect};

    fn setup() -> (Workflow, Catalog, Machines) {
        let catalog = Catalog::bundled().unwrap();
        let mut wf = Workflow::default();
        add_node(&mut wf, "host.ssh", 0.0, 0.0); // n1
        add_node(&mut wf, "automation.custom_script", 300.0, 0.0); // n2
        let mut machines = Machines::default();
        machines.hosts.insert(
            "h1".into(),
            Machine {
                name: "Fedora".into(),
                hue: 3,
                reachable: true,
                state: "Connected".into(),
            },
        );
        (wf, catalog, machines)
    }

    #[test]
    fn cards_say_what_is_missing_and_where_actions_run() {
        let (mut wf, catalog, machines) = setup();
        let none = HashMap::new();
        let view = View::default();
        let host = node_row(&wf, &wf.nodes[0], &catalog, &machines, &none, &view, "");
        assert_eq!(
            (host.detail.as_str(), host.state_text.as_str()),
            ("Not chosen", "Needs setup")
        );
        let action = node_row(&wf, &wf.nodes[1], &catalog, &machines, &none, &view, "n2");
        assert_eq!(action.detail, "Not connected to a machine");
        assert!(action.selected);
        assert_eq!(
            (action.icon.as_str(), action.kind.as_str()),
            ("script", "Automation")
        );

        wf.nodes[0].binding = Some(Binding::Host {
            host_id: "h1".into(),
        });
        connect(&mut wf, &catalog, "n2", "n1").unwrap();
        let host = node_row(&wf, &wf.nodes[0], &catalog, &machines, &none, &view, "");
        assert_eq!(
            (host.tone.as_str(), host.state_text.as_str(), host.hue),
            ("live", "Connected", 3)
        );
        let action = node_row(&wf, &wf.nodes[1], &catalog, &machines, &none, &view, "");
        assert_eq!(
            (action.detail.as_str(), action.state_text.as_str()),
            ("Fedora", "Not run yet")
        );
        // Connected to a host node whose machine is not chosen: it says which node.
        wf.nodes[0].binding = None;
        let action = node_row(&wf, &wf.nodes[1], &catalog, &machines, &none, &view, "");
        assert_eq!(action.detail, "On SSH host");
        wf.nodes[0].binding = Some(Binding::Host {
            host_id: "h1".into(),
        });

        let mut problems = HashMap::new();
        problems.insert(
            "n2".to_string(),
            vec!["Write the apply script.".to_string()],
        );
        let action = node_row(&wf, &wf.nodes[1], &catalog, &machines, &problems, &view, "");
        assert_eq!(
            (action.tone.as_str(), action.state_text.as_str()),
            ("attention", "Needs setup")
        );

        // A host deleted from Hosts is named as such, not silently dropped.
        wf.nodes[0].binding = Some(Binding::Host {
            host_id: "gone".into(),
        });
        let host = node_row(&wf, &wf.nodes[0], &catalog, &machines, &none, &view, "");
        assert_eq!(host.detail, "No longer in the inventory");
    }

    #[test]
    fn runs_on_links_are_drawn_from_the_machine() {
        let (mut wf, catalog, _) = setup();
        connect(&mut wf, &catalog, "n2", "n1").unwrap();
        let view = View {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
        };
        let e = edge_row(&wf, 0, &view);
        assert_eq!(e.kind, "target");
        // Starts at the host's right edge (x 220), ends at the script's left edge (x 300).
        assert!(e.x <= 220.0 && e.x + e.w >= 300.0);
        assert!(e.commands.starts_with("M "));
    }

    #[test]
    fn the_palette_groups_by_category_and_filters() {
        let catalog = Catalog::bundled().unwrap();
        let all = palette_rows(&catalog, "");
        assert!(all[0].header && all[0].name == "Host");
        assert!(all.iter().any(|r| r.id == "automation.custom_script"));
        // Name, summary and category all count, in any case.
        let apk = palette_rows(&catalog, "APK");
        let ids: Vec<&str> = apk
            .iter()
            .filter(|r| !r.header)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(ids, ["deployment.flutter_deploy", "android.install_apk"]);
        let screen = palette_rows(&catalog, "SCREEN");
        assert_eq!(screen.len(), 2);
        assert_eq!(
            (screen[0].name.as_str(), screen[1].id.as_str()),
            ("Android", "android.screenshot")
        );
        assert!(palette_rows(&catalog, "nothing like this").is_empty());
    }

    #[test]
    fn cards_name_their_result_in_the_last_run() {
        assert_eq!(
            last_run_state("succeeded", Some(false)),
            ("live", "Up to date")
        );
        assert_eq!(
            last_run_state("succeeded", Some(true)),
            ("live", "Done in the last run")
        );
        assert_eq!(
            last_run_state("succeeded", None),
            ("live", "Done in the last run")
        );
        assert_eq!(last_run_state("failed", Some(true)).0, "attention");
        assert_eq!(last_run_state("skipped", None).1, "Skipped in the last run");
        assert_eq!(last_run_state("pending", None), ("idle", "Not run yet"));
    }

    #[test]
    fn messages_read_as_sentences() {
        assert_eq!(
            sentence("write the apply script"),
            "Write the apply script."
        );
        assert_eq!(sentence("Done."), "Done.");
        assert_eq!(sentence(""), "");
        assert_eq!(zoom_text(0.67), "67 %");
    }
}
