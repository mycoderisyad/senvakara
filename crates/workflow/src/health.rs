//! Service health (plan WF §5, ADR-033): a module's `health.sh` says whether
//! what it installed is running. All probes of one host go out as one
//! script on stdin of `bash -s`, run as the signed-in user (never sudo,
//! never with secrets), each in its own bounded subshell. Health is checked
//! only when the user asks (opening a workflow, after a run, Refresh).

use crate::graph::{Binding, EdgeKind, Workflow};
use crate::module::Executor;
use crate::params::{check_params, preamble, shell_quote, ParamValue};
use crate::Catalog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Seconds one probe may take beyond the wait it was given.
pub const PROBE_SECS: u32 = 20;
/// Longest wait right after an apply for a service to come up.
pub const AFTER_APPLY_WAIT_SECS: u32 = 90;
/// Longest detail shown for one service.
const MAX_DETAIL: usize = 200;
/// Lines of one probe's output kept; the result is its last line.
const TAIL_LINES: u32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Running,
    Degraded,
    Stopped,
    Unknown,
}

impl HealthState {
    /// The word a card shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Degraded => "Degraded",
            Self::Stopped => "Stopped",
            Self::Unknown => "Unknown",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "running" => Some(Self::Running),
            "degraded" => Some(Self::Degraded),
            "stopped" => Some(Self::Stopped),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub state: HealthState,
    pub detail: String,
}

impl Health {
    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            state: HealthState::Unknown,
            detail: clean(&detail.into()),
        }
    }
}

/// One `health.sh` with what it needs to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub node_id: String,
    /// The host the node runs on.
    pub host_id: String,
    /// `export` lines of the node's settings, secrets left out.
    pub head: String,
    pub body: String,
}

/// The `export` lines a health probe gets: every setting except secrets
/// (`health.sh` never receives them), then how long it may wait.
pub fn probe_head(values: &BTreeMap<String, ParamValue>, wait_secs: u32) -> String {
    let public = crate::secrets::without_secrets(values);
    let mut head = preamble(&public, &BTreeMap::new()).expect("no secrets to resolve");
    head.push_str(&format!(
        "export SV_HEALTH_WAIT={}\n",
        wait_secs.min(AFTER_APPLY_WAIT_SECS)
    ));
    head
}

/// Every service node of the workflow that runs on an SSH host and has a
/// `health.sh`, with the node's own edit of it when there is one. A node
/// whose settings do not check out yet is left out with the reason.
pub fn probes(wf: &Workflow, catalog: &Catalog) -> (Vec<Probe>, Vec<(String, Health)>) {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for node in &wf.nodes {
        let Some(module) = catalog.get(&node.module) else {
            continue;
        };
        if module.manifest.executor != Executor::SshScript {
            continue;
        }
        let body = node
            .scripts
            .get("health.sh")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| module.files.get("health.sh"));
        let Some(body) = body else {
            continue;
        };
        let host = wf
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Target && e.from == node.id)
            .find_map(|e| match wf.node(&e.to).and_then(|n| n.binding.as_ref()) {
                Some(Binding::Host { host_id }) => Some(host_id.clone()),
                _ => None,
            });
        let Some(host_id) = host else {
            continue;
        };
        match check_params(&module.manifest.params, &node.params) {
            Ok(values) => out.push(Probe {
                node_id: node.id.clone(),
                host_id,
                head: probe_head(&values, 0),
                body: body.clone(),
            }),
            Err(problem) => skipped.push((
                node.id.clone(),
                Health::unknown(format!("not checked: {problem}")),
            )),
        }
    }
    (out, skipped)
}

/// One script for `bash -s` that runs each probe in turn and marks its
/// output with its index. A probe that hangs is stopped after
/// [`PROBE_SECS`] plus its wait. Probes read no stdin.
pub fn batch_script(probes: &[(&str, &str)], wait_secs: u32) -> String {
    let limit = PROBE_SECS + wait_secs.min(AFTER_APPLY_WAIT_SECS);
    let mut out =
        String::from("# Senvakara health probes: read-only, as the signed-in user.\nset +e\n");
    for (i, (head, body)) in probes.iter().enumerate() {
        let script = format!("set -euo pipefail\n{head}{body}\n");
        out.push_str(&format!(
            "echo 'SV_NODE {i}'\n\
             timeout -k 2 {limit} bash -c {} sv-health </dev/null 2>&1 | tail -n {TAIL_LINES}\n\
             [ \"${{PIPESTATUS[0]}}\" = 124 ] && echo 'SV_HEALTH unknown the check did not answer within {limit} s'\n",
            shell_quote(&script)
        ));
    }
    out.push_str("echo 'SV_DONE'\n");
    out
}

/// The result of each probe of [`batch_script`], by index. A probe with no
/// `SV_HEALTH` line is unknown, with its last line of output as the detail.
pub fn parse_batch(output: &str, count: usize) -> Vec<Health> {
    let mut sections: Vec<Vec<&str>> = vec![Vec::new(); count];
    let mut seen = vec![false; count];
    let mut current: Option<usize> = None;
    for line in output.lines() {
        if let Some(n) = line.strip_prefix("SV_NODE ") {
            current = n.trim().parse().ok().filter(|i| *i < count);
            if let Some(i) = current {
                seen[i] = true;
            }
            continue;
        }
        if line == "SV_DONE" {
            current = None;
            continue;
        }
        if let Some(i) = current {
            sections[i].push(line);
        }
    }
    sections
        .iter()
        .zip(seen)
        .map(|(lines, seen)| {
            if !seen {
                return Health::unknown("the host did not run this check");
            }
            parse_result(lines)
        })
        .collect()
}

/// The last `SV_HEALTH <state> <detail>` line among one probe's lines.
pub fn parse_result(lines: &[&str]) -> Health {
    let found = lines.iter().rev().find_map(|line| {
        let rest = line.trim().strip_prefix("SV_HEALTH ")?;
        let (word, detail) = rest.split_once(' ').unwrap_or((rest, ""));
        Some(Health {
            state: HealthState::parse(word.trim())?,
            detail: clean(detail),
        })
    });
    found.unwrap_or_else(|| {
        let last = lines
            .iter()
            .rev()
            .find(|l| !l.trim().is_empty())
            .copied()
            .unwrap_or("");
        Health::unknown(if last.is_empty() {
            "health.sh gave no result".to_string()
        } else {
            format!("health.sh gave no result: {last}")
        })
    })
}

/// One line of printable text, bounded.
fn clean(text: &str) -> String {
    let one: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.trim();
    match one.char_indices().nth(MAX_DETAIL) {
        Some((cut, _)) => format!("{}...", &one[..cut]),
        None => one.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, Node};

    #[test]
    fn the_last_health_line_wins_and_odd_output_is_unknown() {
        let h = parse_result(&[
            "starting",
            "SV_HEALTH stopped not yet",
            "SV_HEALTH running port 80 answers",
        ]);
        assert_eq!(h.state, HealthState::Running);
        assert_eq!(h.detail, "port 80 answers");
        let h = parse_result(&["SV_HEALTH great all fine", "permission denied"]);
        assert_eq!(h.state, HealthState::Unknown);
        assert!(h.detail.contains("permission denied"), "{}", h.detail);
        assert_eq!(parse_result(&[]).detail, "health.sh gave no result");
        let long = format!("SV_HEALTH degraded {}\u{1b}[31m", "x".repeat(500));
        let h = parse_result(&[&long]);
        assert_eq!(h.state, HealthState::Degraded);
        assert!(h.detail.len() <= MAX_DETAIL + 3 && !h.detail.contains('\u{1b}'));
    }

    #[test]
    fn a_batch_keeps_each_probe_apart_and_quotes_every_script() {
        let script = batch_script(
            &[("export SV_PORT=80\n", "echo 'it'\\''s'"), ("", "exit 3")],
            5,
        );
        assert!(script.contains("echo 'SV_NODE 0'\n") && script.contains("echo 'SV_NODE 1'\n"));
        assert!(script.contains("timeout -k 2 25 bash -c 'set -euo pipefail\nexport SV_PORT=80\n"));
        assert!(script.contains("</dev/null"));
        assert!(script.ends_with("echo 'SV_DONE'\n"));
        let output = "SV_NODE 0\nhello\nSV_HEALTH running ok\nSV_NODE 1\nSV_DONE\n";
        let got = parse_batch(output, 3);
        assert_eq!(got[0].state, HealthState::Running);
        assert_eq!(got[1].state, HealthState::Unknown);
        assert_eq!(got[2].detail, "the host did not run this check");
        // A marker outside the probes' range is ignored.
        assert_eq!(
            parse_batch("SV_NODE 9\nSV_HEALTH running x\n", 1)[0].state,
            HealthState::Unknown
        );
    }

    #[test]
    fn probes_get_no_secrets_and_a_bounded_wait() {
        let mut values = BTreeMap::new();
        values.insert("port".to_string(), ParamValue::Int(3001));
        values.insert(
            "token".to_string(),
            ParamValue::SecretRef("secret-1".into()),
        );
        let head = probe_head(&values, 600);
        assert!(head.contains("export SV_PORT=3001\n"));
        assert!(!head.contains("SV_TOKEN") && !head.contains("secret-1"));
        assert!(head.ends_with(&format!("export SV_HEALTH_WAIT={AFTER_APPLY_WAIT_SECS}\n")));
    }

    #[test]
    fn only_service_nodes_on_a_host_are_probed() {
        let catalog = Catalog::bundled().unwrap();
        let mut wf = Workflow::default();
        let node = |id: &str, module: &str| -> Node {
            serde_json::from_value(serde_json::json!({ "id": id, "module": module })).unwrap()
        };
        let mut host = node("h", "host.ssh");
        host.binding = Some(Binding::Host {
            host_id: "host-1".into(),
        });
        wf.nodes.push(host);
        let mut service = node("f", "security.fail2ban");
        service
            .scripts
            .insert("health.sh".into(), "echo 'SV_HEALTH running edited'".into());
        wf.nodes.push(service);
        let mut broken = node("b", "security.fail2ban");
        broken.scripts.insert("health.sh".into(), "true".into());
        broken
            .params
            .insert("max_retry".into(), serde_json::json!(999));
        wf.nodes.push(broken);
        wf.nodes.push(node("w", "automation.wait"));
        wf.nodes.push(node("loose", "security.fail2ban"));
        for from in ["f", "b", "w"] {
            wf.edges.push(Edge {
                from: from.into(),
                to: "h".into(),
                kind: EdgeKind::Target,
            });
        }
        let (probes, skipped) = probes(&wf, &catalog);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].node_id, "f");
        assert_eq!(probes[0].host_id, "host-1");
        assert_eq!(probes[0].body, "echo 'SV_HEALTH running edited'");
        assert!(probes[0].head.contains("SV_MAX_RETRY=5"));
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, "b");
        assert!(skipped[0].1.detail.starts_with("not checked:"));
    }
}
