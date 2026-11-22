//! Keeps the panels of open workspaces in line with what can run, starting their sessions.

use std::rc::Rc;

use senvakara_adapters_pty::ShellKind;

use crate::inventory::transport_label;
use crate::{App, Pane};

use super::availability::blocked;
use super::{Shell, Spec};

impl App {
    /// Hosts or phones changed: panels start or explain themselves, pickers follow.
    pub fn workspaces_inventory_changed(self: &Rc<Self>) {
        let tabs: Vec<usize> = self
            .sessions
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, pane)| matches!(pane, Pane::Workspace(_)))
            .map(|(tab, _)| tab)
            .collect();
        for tab in tabs {
            self.workspace_sync(tab);
        }
        self.workspaces_list_publish();
    }

    /// Bring the panels of a workspace in line with what can run: start the ones that can and
    /// have no session, and drop ended sessions whose target went away so the panel says why.
    pub(super) fn workspace_sync(self: &Rc<Self>, tab: usize) {
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let plan = self.with_workspace(tab, |pane| {
            let mut start = Vec::new();
            let mut slots = pane.slots.borrow_mut();
            for (index, slot) in slots.iter_mut().enumerate() {
                let Some(spec) = &slot.spec else { continue };
                let reason = blocked(spec, &hosts, &devices);
                match (&slot.pane, reason) {
                    (None, None) => start.push((index, spec.clone())),
                    (Some(live), Some(_)) if !live.running() => slot.pane = None,
                    _ => {}
                }
            }
            start
        });
        for (index, spec) in plan.unwrap_or_default() {
            let Some(live) = self.start_panel(&spec) else {
                continue;
            };
            let id = live.id();
            let placed = self.with_workspace(tab, |pane| {
                let mut slots = pane.slots.borrow_mut();
                match slots.get_mut(index) {
                    Some(slot) if slot.spec.as_ref() == Some(&spec) && slot.pane.is_none() => {
                        slot.pane = Some(live);
                        true
                    }
                    _ => false,
                }
            });
            if placed == Some(true) {
                self.pane_started(id);
            }
        }
        self.workspace_publish(tab);
    }

    /// Start the session a panel describes. The caller has checked that it can run.
    fn start_panel(self: &Rc<Self>, spec: &Spec) -> Option<Pane> {
        let id = self.take_id();
        let (cols, lines) = self.grid.get();
        match spec {
            Spec::Terminal { shell, .. } => match shell {
                Shell::Windows | Shell::Wsl(_) => {
                    let kind = match shell {
                        Shell::Wsl(distro) => ShellKind::Wsl {
                            distro: distro.clone(),
                        },
                        _ => ShellKind::PowerShell,
                    };
                    match crate::term::Session::spawn(id, &kind, cols, lines, crate::notifier(id)) {
                        Ok(session) => Some(Pane::Term(session)),
                        Err(error) => {
                            // A terminal that says why it could not start, instead of nothing.
                            let (session, feeder, _commands) =
                                crate::term::Session::remote(id, cols, lines, crate::notifier(id));
                            feeder.feed(
                                format!("The shell could not be started: {error:#}\r\n").as_bytes(),
                            );
                            feeder.end();
                            Some(Pane::Term(session))
                        }
                    }
                }
                Shell::Ssh { host_id, tmux } => {
                    let host = self.inv.host(host_id)?;
                    Some(Pane::Term(self.start_ssh(id, &host, tmux.clone())))
                }
            },
            Spec::Mirror { route_id } | Spec::Logcat { route_id } => {
                let device = self.inv.device(route_id)?;
                let record = &device.record;
                let route = format!(
                    "{}  >  {}",
                    record.host_name,
                    transport_label(&record.connection_type)
                );
                let hue = self.inv.hue(&record.host_id);
                Some(if matches!(spec, Spec::Mirror { .. }) {
                    Pane::Mirror(crate::mirror::MirrorPane::start(
                        id,
                        self.backend.clone(),
                        record.route_id.clone(),
                        record.display_name.clone(),
                        route,
                        hue,
                        crate::notifier(id),
                    ))
                } else {
                    Pane::Logcat(crate::logcat::LogcatPane::start(
                        id,
                        self.backend.clone(),
                        record.route_id.clone(),
                        record.display_name.clone(),
                        route,
                        hue,
                        crate::notifier(id),
                    ))
                })
            }
            Spec::Desktop { host_id } => {
                let host = self.inv.host(host_id)?;
                Some(Pane::Desktop(crate::desktop::DesktopPane::start(
                    id,
                    self.backend.clone(),
                    host.id.clone(),
                    host.display_name.clone(),
                    self.inv.hue(&host.id),
                    crate::notifier(id),
                )))
            }
            Spec::Files { host_id } => Some(Pane::Files(crate::files::FilesPane::new(
                id,
                crate::files::Source::Host(host_id.clone()),
            ))),
            Spec::Unavailable { .. } => None,
        }
    }
}
