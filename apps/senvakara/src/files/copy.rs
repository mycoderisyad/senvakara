//! Copying the selected files of one side to the other through the job list.

use std::rc::Rc;

use crate::device::target_facts;
use crate::inventory::fact;
use crate::jobs::JobSpec;
use crate::{App, Pane, Pending, Question};

use super::entries::Entry;
use super::paths::{join, same_dir};
use super::{pair_supported, Kind, Source};

/// Why the selection of one side cannot be copied to the other, or "" when it can.
pub(super) fn copy_problem(
    from: Kind,
    to: Kind,
    selected: &[&Entry],
    to_loaded: bool,
    to_path: &str,
    phone_ready: bool,
) -> &'static str {
    if !pair_supported(from, to) {
        return match from {
            Kind::Local => "Not supported yet: both sides show this PC.",
            Kind::Host => "Not supported yet: copying from one host to another.",
            Kind::Device => "Not supported yet: copying from one phone to another.",
        };
    }
    if !phone_ready {
        return "The phone must be connected and authorized first.";
    }
    if !to_loaded {
        return "The other side has no folder open yet.";
    }
    if to == Kind::Local && to_path.is_empty() {
        return "Open a folder on the other side first. Files cannot be copied to the list of drives.";
    }
    if selected.is_empty() {
        return "Select one or more files first.";
    }
    if selected.iter().any(|entry| entry.is_dir || entry.is_drive) {
        return "Folders cannot be copied yet. Select files only.";
    }
    ""
}

impl App {
    /// Whether the phone of a side (if it shows one) can be used right now.
    pub(super) fn phone_ready(&self, source: &Source) -> bool {
        match source {
            Source::Device(route) => self
                .inv
                .device(route)
                .is_some_and(|device| device.record.state == "available"),
            _ => true,
        }
    }

    /// Copy the selected files of `side` to the folder on the other side. A copy to or from a
    /// phone is confirmed first, naming the phone, its route and both folders.
    pub(super) fn files_copy(self: &Rc<Self>, id: i32, side: usize) {
        if side > 1 {
            return;
        }
        let plan = self
            .with_files(id, |pane| {
                let from = pane.sides[side].borrow();
                let to = pane.sides[1 - side].borrow();
                let selected: Vec<&Entry> = from
                    .selected
                    .iter()
                    .filter_map(|row| from.entries.get(*row))
                    .collect();
                let problem = copy_problem(
                    from.source.kind(),
                    to.source.kind(),
                    &selected,
                    to.loaded,
                    &to.path,
                    self.phone_ready(&from.source) && self.phone_ready(&to.source),
                );
                if !problem.is_empty() {
                    return None;
                }
                let names: Vec<String> = selected.iter().map(|entry| entry.name.clone()).collect();
                Some((
                    from.source.clone(),
                    from.path.clone(),
                    to.source.clone(),
                    to.path.clone(),
                    names,
                ))
            })
            .flatten();
        let Some((from, from_dir, to, to_dir, names)) = plan else {
            return;
        };
        let (from_name, to_name) = (self.place_name(&from), self.place_name(&to));
        let phone = match (&from, &to) {
            (Source::Device(route), _) | (_, Source::Device(route)) => {
                let Some(device) = self.inv.device(route) else {
                    return;
                };
                Some(device)
            }
            _ => None,
        };
        let serial = phone
            .as_ref()
            .map(|device| device.record.serial.clone())
            .unwrap_or_default();

        let mut jobs = Vec::new();
        for name in &names {
            let path = join(from.kind(), &from_dir, name);
            let spec = match (&from, &to) {
                (Source::Local, Source::Host(host)) => JobSpec::SftpUpload {
                    host_id: host.clone(),
                    local_path: path,
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Host(host), Source::Local) => JobSpec::SftpDownload {
                    host_id: host.clone(),
                    remote_path: path,
                    local_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Local, Source::Device(route)) => JobSpec::DevicePush {
                    route_id: route.clone(),
                    serial: serial.clone(),
                    local_path: path,
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Device(route), Source::Local) => JobSpec::DevicePull {
                    route_id: route.clone(),
                    remote_path: path,
                    local_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Host(host), Source::Device(route)) => JobSpec::HostToDevice {
                    host_id: host.clone(),
                    remote_path: path,
                    route_id: route.clone(),
                    serial: serial.clone(),
                    device_dir: to_dir.clone(),
                    overwrite: false,
                },
                (Source::Device(route), Source::Host(host)) => JobSpec::DeviceToHost {
                    route_id: route.clone(),
                    device_path: path,
                    host_id: host.clone(),
                    remote_dir: to_dir.clone(),
                    overwrite: false,
                },
                _ => return,
            };
            jobs.push((copy_label(name, &from_name, &to_name), spec));
        }

        let Some(device) = phone else {
            for (label, spec) in jobs {
                self.job_start(label, spec);
            }
            return;
        };
        let mut facts = target_facts(&device);
        let files = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} files: {}", names.len(), names.join(", "))
        };
        facts.push(fact(
            if names.len() == 1 { "File" } else { "Files" },
            files,
            true,
        ));
        facts.push(fact("From", format!("{from_name}: {from_dir}"), true));
        facts.push(fact("To", format!("{to_name}: {to_dir}"), true));
        let to_phone = matches!(to, Source::Device(_));
        self.ask_with(
            Question {
                title: if to_phone {
                    format!("Copy to {}?", device.record.display_name)
                } else {
                    format!("Copy from {}?", device.record.display_name)
                },
                body: if to_phone {
                    "The files are written to this phone only. A file with the same name is not replaced without asking."
                } else {
                    "The files are read from this phone only. A file with the same name is not replaced without asking."
                }
                .into(),
                confirm: "Copy".into(),
                danger: false,
            },
            facts,
            "",
            Pending::CopyFiles(jobs),
        );
    }

    /// A copy ended well: show the new file wherever its folder is on screen.
    pub fn files_job_finished(self: &Rc<Self>, spec: &JobSpec) {
        let Some((source, dir)) = spec.destination() else {
            return;
        };
        let mut stale = Vec::new();
        self.each_pane(|pane, _| {
            let Pane::Files(pane) = pane else { return };
            for (side, state) in pane.sides.iter().enumerate() {
                let state = state.borrow();
                if state.loaded
                    && state.source == source
                    && same_dir(source.kind(), &state.path, &dir)
                {
                    stale.push((pane.id, side, state.path.clone()));
                }
            }
        });
        for (id, side, path) in stale {
            self.files_load(id, side, source.clone(), path);
        }
    }
}

/// What the job list calls one copy.
fn copy_label(name: &str, from: &str, to: &str) -> String {
    format!("Copy {name} from {from} to {to}")
}
