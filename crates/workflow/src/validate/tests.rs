use super::*;
use crate::graph::{Edge, Permissions};
use crate::plan;
use serde_json::json;

fn catalog() -> Catalog {
    Catalog::bundled().unwrap()
}

fn node(id: &str, module: &str) -> Node {
    Node {
        id: id.into(),
        module: module.into(),
        title: String::new(),
        x: 0.0,
        y: 0.0,
        params: BTreeMap::new(),
        binding: None,
        scripts: BTreeMap::new(),
    }
}

fn host(id: &str, host_id: &str) -> Node {
    let mut n = node(id, "host.ssh");
    n.binding = Some(Binding::Host {
        host_id: host_id.into(),
    });
    n
}

fn script(id: &str, apply: &str) -> Node {
    let mut n = node(id, "automation.custom_script");
    n.scripts.insert("apply.sh".into(), apply.into());
    n
}

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
    }
}

fn shell_only() -> Permissions {
    Permissions {
        shell: true,
        ..Permissions::default()
    }
}

fn messages(wf: &Workflow) -> Vec<String> {
    validate(wf, &catalog())
        .unwrap_err()
        .into_iter()
        .map(|p| p.message)
        .collect()
}

#[test]
fn an_action_must_run_on_exactly_one_suitable_machine() {
    let mut wf = Workflow {
        nodes: vec![
            host("h1", "host-1"),
            host("h2", "host-2"),
            script("s", "echo hi"),
        ],
        permissions: shell_only(),
        ..Workflow::default()
    };
    assert!(messages(&wf)[0].contains("connect this node to the host"));

    wf.edges = vec![
        edge("s", "h1", EdgeKind::Target),
        edge("s", "h2", EdgeKind::Target),
    ];
    assert!(messages(&wf)[0].contains("one host or device only"));

    wf.edges = vec![edge("s", "h1", EdgeKind::Target)];
    let checked = validate(&wf, &catalog()).unwrap();
    assert_eq!(
        checked[0].target,
        Binding::Host {
            host_id: "host-1".into()
        }
    );
}

#[test]
fn connections_are_typed() {
    let mut phone = node("p", "android.device");
    phone.binding = Some(Binding::Device {
        route_id: "r".into(),
    });
    let wf = Workflow {
        nodes: vec![host("h", "host-1"), phone, script("s", "echo hi")],
        edges: vec![
            // A bash script cannot run on a phone.
            edge("s", "p", EdgeKind::Target),
            // Order connections never touch anchors.
            edge("h", "s", EdgeKind::After),
            // A target goes from the action to the anchor, not back.
            edge("h", "s", EdgeKind::Target),
        ],
        permissions: shell_only(),
        ..Workflow::default()
    };
    let m = messages(&wf);
    assert!(
        m.iter()
            .any(|m| m.contains("cannot run on an Android device")),
        "{m:?}"
    );
    assert!(m.iter().any(|m| m.contains("link two actions")), "{m:?}");
    assert!(
        m.iter().any(|m| m.contains("from an action to a host")),
        "{m:?}"
    );
}

#[test]
fn anchors_need_the_right_kind_of_machine() {
    let mut wrong = node("h", "host.ssh");
    wrong.binding = Some(Binding::LocalPc);
    let mut stray = script("s", "echo");
    stray.binding = Some(Binding::LocalPc);
    let wf = Workflow {
        nodes: vec![wrong, stray],
        ..Workflow::default()
    };
    let m = messages(&wf);
    assert!(
        m.iter().any(|m| m == "choose an SSH host for this node"),
        "{m:?}"
    );
    assert!(
        m.iter().any(|m| m.contains("only host and device nodes")),
        "{m:?}"
    );
}

#[test]
fn loops_are_reported_with_their_nodes() {
    let wf = Workflow {
        nodes: vec![
            host("h", "host-1"),
            script("a", "1"),
            script("b", "2"),
            script("c", "3"),
        ],
        edges: vec![
            edge("a", "h", EdgeKind::Target),
            edge("b", "h", EdgeKind::Target),
            edge("c", "h", EdgeKind::Target),
            edge("a", "b", EdgeKind::After),
            edge("b", "c", EdgeKind::DependsOn),
            edge("c", "b", EdgeKind::After),
        ],
        permissions: shell_only(),
        ..Workflow::default()
    };
    let m = messages(&wf);
    assert_eq!(m, ["these nodes wait for each other in a loop: b, c"]);
}

#[test]
fn permissions_gate_modules_and_the_root_switch() {
    let mut as_root = script("s", "apt-get update");
    as_root.params.insert("as_root".into(), json!(true));
    let mut wf = Workflow {
        nodes: vec![host("h", "host-1"), as_root],
        edges: vec![edge("s", "h", EdgeKind::Target)],
        ..Workflow::default()
    };
    let m = messages(&wf);
    assert!(m.iter().any(|m| m.contains("run shell commands")), "{m:?}");
    assert!(m.iter().any(|m| m.contains("run as root (sudo)")), "{m:?}");

    wf.permissions = Permissions {
        shell: true,
        root: true,
        ..Permissions::default()
    };
    let checked = validate(&wf, &catalog()).unwrap();
    assert!(checked[0].needs_root);
}

#[test]
fn scripts_are_bounded_and_only_where_a_module_has_them() {
    let big = script("big", &"x".repeat(MAX_SCRIPT_LEN + 1));
    let mut odd = script("odd", "echo");
    odd.scripts.insert("evil.sh".into(), "x".into());
    let mut phone = node("p", "android.device");
    phone.binding = Some(Binding::Device {
        route_id: "r".into(),
    });
    let mut apk = node("a", "android.install_apk");
    apk.params.insert("path".into(), json!("C:\\a.apk"));
    apk.scripts.insert("apply.sh".into(), "x".into());
    let empty = script("e", "   ");
    let wf = Workflow {
        nodes: vec![host("h", "host-1"), phone, big, odd, apk, empty],
        edges: vec![
            edge("big", "h", EdgeKind::Target),
            edge("odd", "h", EdgeKind::Target),
            edge("a", "p", EdgeKind::Target),
            edge("e", "h", EdgeKind::Target),
        ],
        permissions: Permissions {
            shell: true,
            install: true,
            ..Permissions::default()
        },
        ..Workflow::default()
    };
    let m = messages(&wf);
    assert!(
        m.iter().any(|m| m == "apply.sh is larger than 64 KiB"),
        "{m:?}"
    );
    assert!(m.iter().any(|m| m == "unexpected script evil.sh"), "{m:?}");
    assert!(
        m.iter().any(|m| m == "this node has no scripts to edit"),
        "{m:?}"
    );
    assert!(m.iter().any(|m| m == "write the apply script"), "{m:?}");
}

#[test]
fn bad_settings_and_unknown_modules_point_at_their_node() {
    let mut wait = node("w", "automation.wait");
    wait.params.insert("seconds".into(), json!(0));
    let wf = Workflow {
        nodes: vec![host("h", "host-1"), wait, node("x", "teleport.now")],
        edges: vec![edge("w", "h", EdgeKind::Target)],
        ..Workflow::default()
    };
    let problems = validate(&wf, &catalog()).unwrap_err();
    assert!(problems
        .iter()
        .any(|p| p.node.as_deref() == Some("w") && p.message.contains("between 1 and 300")));
    assert!(problems
        .iter()
        .any(|p| p.node.as_deref() == Some("x") && p.message.contains("unknown node type")));
}

#[test]
fn the_plan_orders_by_connections_and_groups_lanes_per_machine() {
    let wf = Workflow {
        nodes: vec![
            host("h1", "host-1"),
            host("h2", "host-2"),
            script("db", "install db"),
            script("app", "install app"),
            script("proxy", "install proxy"),
            script("other", "on host 2"),
        ],
        edges: vec![
            edge("db", "h1", EdgeKind::Target),
            edge("app", "h1", EdgeKind::Target),
            edge("proxy", "h1", EdgeKind::Target),
            edge("other", "h2", EdgeKind::Target),
            // proxy after app, app uses db: added in a different order.
            edge("app", "proxy", EdgeKind::After),
            edge("db", "app", EdgeKind::DependsOn),
        ],
        permissions: shell_only(),
        ..Workflow::default()
    };
    let p = plan(&wf, &catalog()).unwrap();
    let ids: Vec<&str> = p.steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["db", "app", "proxy", "other"]);
    assert_eq!(p.waits_for, [vec![], vec![0], vec![1], vec![]]);
    let lanes = p.lanes();
    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0].1, [0, 1, 2]);
    assert_eq!(lanes[1].1, [3]);
    assert_eq!(p.downstream(0), [1, 2].into());
    assert!(p.downstream(3).is_empty());
    assert!(!p.needs_root());
}

#[test]
fn a_stored_graph_round_trips_through_json() {
    let wf = Workflow {
        nodes: vec![host("h", "host-1"), script("s", "echo")],
        edges: vec![edge("s", "h", EdgeKind::Target)],
        permissions: shell_only(),
        ..Workflow::default()
    };
    let text = serde_json::to_string(&wf).unwrap();
    let back: Workflow = serde_json::from_str(&text).unwrap();
    assert_eq!(back, wf);
    // An empty object is a valid, empty workflow of the current version.
    let empty: Workflow = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, Workflow::default());
}
