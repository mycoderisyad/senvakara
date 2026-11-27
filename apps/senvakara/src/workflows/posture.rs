//! Security posture on the canvas (plan WF §5): the "Hardened N/M" badge on SSH host cards,
//! the Security section of a host node with each control, its evidence and a Fix that adds the
//! module setting it right, and the "New VPS baseline" template. The read-only check runs when
//! the user opens a workflow (connected hosts whose result is older than five minutes), after a
//! run, and on Refresh; never on a timer. It is a hardening overview, not a compliance audit.

use std::rc::Rc;

use senvakara_app::workflow::{
    add_vps_baseline, control_info, Binding, Edge, EdgeKind, Node, Posture, Status,
};
use senvakara_app::PostureView;

use super::edit;
use super::geometry::{NODE_H, NODE_W};
use crate::{App, WfControl};

/// A cached result older than this is checked again when the workflow opens.
const STALE_SECS: i64 = 300;
/// Room between a host card and the steps added under it, in world units.
const BELOW: f32 = 150.0;

/// The card badge: passing controls over controls that count.
pub fn badge(posture: &Posture) -> String {
    let (pass, all) = posture.score();
    format!("Hardened {pass}/{all}")
}

/// "just now", "4 min ago", "2 h ago", "3 days ago".
pub fn age_text(secs: i64) -> String {
    match secs {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => match secs / 86_400 {
            1 => "1 day ago".into(),
            days => format!("{days} days ago"),
        },
    }
}

/// Whether opening a workflow should check this host again.
pub fn is_stale(cached: Option<&PostureView>) -> bool {
    cached.is_none_or(|p| p.age_secs > STALE_SECS)
}

pub fn control_rows(posture: &Posture) -> Vec<WfControl> {
    posture
        .controls
        .iter()
        .filter_map(|c| {
            let info = control_info(&c.id)?;
            let (tone, word) = match c.status {
                Status::Pass => ("live", "Passes"),
                Status::Fail => ("attention", "Fails"),
                Status::Unknown => ("idle", "Unknown"),
                Status::Info => ("idle", "Info"),
            };
            let fixable = info.fix.is_some() && matches!(c.status, Status::Fail | Status::Info);
            Some(WfControl {
                id: c.id.clone().into(),
                label: info.label.into(),
                tone: tone.into(),
                state_text: word.into(),
                evidence: c.evidence.clone().into(),
                fixable,
            })
        })
        .collect()
}

/// The host an SSH host node points at.
fn host_of(node: &Node) -> Option<&str> {
    match (&node.binding, node.module.as_str()) {
        (Some(Binding::Host { host_id }), "host.ssh") => Some(host_id),
        _ => None,
    }
}

impl App {
    /// Reads every cached posture (fast: the database only).
    pub(super) fn workflows_postures_load(&self) {
        let cached = self.backend.host_postures().unwrap_or_default();
        *self.wf.postures.borrow_mut() =
            cached.into_iter().map(|p| (p.host_id.clone(), p)).collect();
    }

    /// The badge of an SSH host node, when its host was checked.
    pub(super) fn workflows_badge(&self, node: &Node) -> Option<String> {
        let host = host_of(node)?;
        self.wf
            .postures
            .borrow()
            .get(host)
            .map(|p| badge(&p.posture))
    }

    /// Checks the connected hosts of the open workflow: those whose result is stale, or all of
    /// them after a run (`all`). Hosts that are not connected are left alone: opening a
    /// workflow never opens connections.
    pub(super) fn workflows_postures_auto(self: &Rc<Self>, all: bool) {
        let hosts: Vec<String> = match self.wf.open.borrow().as_ref() {
            Some(o) => o
                .graph
                .nodes
                .iter()
                .filter_map(host_of)
                .map(str::to_string)
                .collect(),
            None => return,
        };
        for host in hosts {
            let connected = self
                .wf
                .machines
                .borrow()
                .hosts
                .get(&host)
                .is_some_and(|m| m.reachable);
            let stale = is_stale(self.wf.postures.borrow().get(&host));
            if connected && (all || stale) {
                self.workflows_posture_check(host);
            }
        }
    }

    /// Refresh in the Security section: check the selected host now (connecting if needed).
    pub(super) fn workflows_posture_refresh(self: &Rc<Self>) {
        let host = {
            let open = self.wf.open.borrow();
            let selected = self.wf.selected.borrow();
            open.as_ref()
                .and_then(|o| o.graph.node(&selected))
                .and_then(host_of)
                .map(str::to_string)
        };
        if let Some(host) = host {
            self.workflows_posture_check(host);
        }
    }

    fn workflows_posture_check(self: &Rc<Self>, host: String) {
        if !self.wf.checking.borrow_mut().insert(host.clone()) {
            return;
        }
        self.workflows_fill_inspector(false);
        let backend = self.backend.clone();
        let id = host.clone();
        self.run(
            async move { backend.host_posture_check(id).await },
            move |app, result| {
                app.wf.checking.borrow_mut().remove(&host);
                match result {
                    Ok(view) => {
                        app.wf.posture_errors.borrow_mut().remove(&host);
                        app.wf.postures.borrow_mut().insert(host, view);
                    }
                    Err(error) => {
                        app.wf
                            .posture_errors
                            .borrow_mut()
                            .insert(host, crate::inventory::friendly(&error));
                    }
                }
                app.workflows_refresh(false);
            },
        );
    }

    /// Fills the Security section for the selected node (only SSH host nodes with a host).
    pub(super) fn workflows_fill_security(&self, node: &Node) {
        let Some(host) = host_of(node) else {
            self.with_view(|v| v.set_security_shown(false));
            return;
        };
        let busy = self.wf.checking.borrow().contains(host);
        let error = self.wf.posture_errors.borrow().get(host).cloned();
        let postures = self.wf.postures.borrow();
        let cached = postures.get(host);
        let (score, rows) = match cached {
            Some(p) => (badge(&p.posture), control_rows(&p.posture)),
            None => ("Not checked yet".to_string(), Vec::new()),
        };
        let state = match (busy, &error, cached) {
            (true, _, _) => "Checking... read-only, without sudo.".to_string(),
            (false, Some(error), _) => format!("The last check did not finish: {error}"),
            (false, None, Some(p)) => format!(
                "Checked {}, read-only and without sudo. A hardening overview, not a compliance audit.",
                age_text(p.age_secs)
            ),
            (false, None, None) => {
                "Refresh runs a read-only check as the signed-in user (no sudo, nothing changes)."
                    .to_string()
            }
        };
        drop(postures);
        self.wf.security.set_vec(rows);
        self.with_view(|v| {
            v.set_security_shown(true);
            v.set_security_busy(busy);
            v.set_security_score(score.into());
            v.set_security_state(state.into());
        });
    }

    /// Fix: adds the module that sets the control right, running on the selected host node,
    /// and selects it. A node of that module already on this host is selected instead.
    pub(super) fn workflows_posture_fix(self: &Rc<Self>, control: &str) {
        let Some(module) = control_info(control).and_then(|c| c.fix) else {
            return;
        };
        let host_node = self.wf.selected.borrow().clone();
        let existing = self.wf.open.borrow().as_ref().and_then(|o| {
            o.graph
                .edges
                .iter()
                .filter(|e| e.kind == EdgeKind::Target && e.to == host_node)
                .find(|e| o.graph.node(&e.from).is_some_and(|n| n.module == module))
                .map(|e| e.from.clone())
        });
        if let Some(id) = existing {
            self.workflows_select(&id);
            return;
        }
        let mut added = String::new();
        self.workflows_edit(|wf| {
            let Some(host) = wf.node(&host_node) else {
                return;
            };
            let (x, y) = (host.x, host.y);
            let on_host = wf
                .edges
                .iter()
                .filter(|e| e.kind == EdgeKind::Target && e.to == host_node)
                .count();
            let id = edit::add_node(wf, module, x + on_host as f32 * (NODE_W + 40.0), y + BELOW);
            wf.edges.push(Edge {
                from: id.clone(),
                to: host_node.clone(),
                kind: EdgeKind::Target,
            });
            added = id;
        });
        if !added.is_empty() {
            self.workflows_select(&added);
        }
    }

    /// Adds a ready-made workflow. The baseline joins the selected SSH host node, else the only
    /// one on the canvas, else a new host node in the middle of the view.
    pub(super) fn workflows_add_template(self: &Rc<Self>, id: &str) {
        if id != "vps_baseline" {
            return;
        }
        let target = {
            let open = self.wf.open.borrow();
            let Some(o) = open.as_ref() else { return };
            let selected = self.wf.selected.borrow();
            let hosts: Vec<&Node> = o
                .graph
                .nodes
                .iter()
                .filter(|n| n.module == "host.ssh")
                .collect();
            match hosts.iter().find(|n| n.id == *selected) {
                Some(n) => Some(n.id.clone()),
                None if hosts.len() == 1 => Some(hosts[0].id.clone()),
                None => None,
            }
        };
        let view = self.wf.view.get();
        let (w, h) = self.wf.size.get();
        let (cx, cy) = view.to_world(w / 4.0, h / 4.0);
        let mut result = Ok(Vec::new());
        self.workflows_edit(|wf| {
            result = add_vps_baseline(wf, target.as_deref(), cx - NODE_W / 2.0, cy - NODE_H / 2.0);
        });
        match result {
            Ok(added) => {
                // A new host first needs its machine; otherwise start at the first step.
                if let Some(first) = added.first() {
                    self.workflows_select(first);
                }
            }
            Err(error) => self.workflows_error(&super::view::sentence(&error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "SV_POSTURE\troot_login\tpass\tPermitRootLogin no\n\
        SV_POSTURE\tpassword_auth\tfail\tPasswordAuthentication yes\n\
        SV_POSTURE\treboot_required\tfail\trunning kernel 6.1, newest installed 6.8\n\
        SV_POSTURE\tlistening_ports\tinfo\treachable from outside: 22 80\n";

    #[test]
    fn the_badge_counts_passing_controls() {
        let p = Posture::parse(REPORT).unwrap();
        assert_eq!(badge(&p), "Hardened 1/9");
    }

    #[test]
    fn rows_say_the_state_in_words_and_offer_fix_only_where_a_module_helps() {
        let rows = control_rows(&Posture::parse(REPORT).unwrap());
        let row = |id: &str| rows.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(row("root_login").state_text, "Passes");
        assert!(!row("root_login").fixable);
        assert_eq!(row("password_auth").state_text, "Fails");
        assert_eq!(row("password_auth").tone, "attention");
        assert!(row("password_auth").fixable);
        // A reboot is the user's call: no module, no Fix.
        assert!(!row("reboot_required").fixable);
        assert_eq!(row("time_sync").state_text, "Unknown");
        assert!(!row("time_sync").fixable);
        assert!(row("listening_ports").fixable);
    }

    #[test]
    fn ages_read_as_words_and_old_results_are_checked_again() {
        assert_eq!(age_text(5), "just now");
        assert_eq!(age_text(240), "4 min ago");
        assert_eq!(age_text(7200), "2 h ago");
        assert_eq!(age_text(86_400), "1 day ago");
        assert_eq!(age_text(3 * 86_400), "3 days ago");
        let view = |age_secs| PostureView {
            host_id: "h".into(),
            checked_at: String::new(),
            age_secs,
            posture: Posture::default(),
        };
        assert!(is_stale(None));
        assert!(!is_stale(Some(&view(120))));
        assert!(is_stale(Some(&view(301))));
    }
}
