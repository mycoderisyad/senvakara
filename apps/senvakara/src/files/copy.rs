//! Copying files and folders from one place to another through the job list: the selection of
//! a side to the other side, to a side of another pane, or into a folder of a list.

use std::rc::Rc;

use crate::device::target_facts;
use crate::inventory::fact;
use crate::jobs::JobSpec;
use crate::{App, Pane, Pending, Question};

use super::entries::Entry;
use super::paths::{join, same_dir};
use super::{pair_supported, Kind, Source};

/// What a copy is about, besides the selection itself.
#[derive(Clone, Copy, Debug)]
pub(super) struct Route<'a> {
    pub from: Kind,
    pub to: Kind,
    /// The destination has a listing on screen.
    pub to_loaded: bool,
    /// The folder the copy writes to.
    pub to_path: &'a str,
    /// Every phone involved is connected and authorized.
    pub phone_ready: bool,
    /// Source and destination are the same folder of the same place.
    pub same_folder: bool,
}

/// Why the selection of one place cannot be copied to another, or "" when it can.
pub(super) fn copy_problem(route: Route, selected: &[&Entry]) -> &'static str {
    let Route { from, to, .. } = route;
    if from == Kind::None || to == Kind::None {
        return "Select a host or a phone on the other side first.";
    }
    if !pair_supported(from, to) {
        return match from {
            Kind::Device => "Not supported yet: copying from one phone to another.",
            _ => "Not supported yet: both sides show this PC.",
        };
    }
    if !route.phone_ready {
        return "The phone must be connected and authorized first.";
    }
    if !route.to_loaded {
        return "The other side has no folder open yet.";
    }
    if to == Kind::Local && route.to_path.is_empty() {
        return "Open a folder on the other side first. Files cannot be copied to the list of drives.";
    }
    if selected.is_empty() {
        return "Select one or more files or folders first.";
    }
    if selected.iter().any(|entry| entry.is_drive) {
        return "A drive cannot be copied. Open it and select what is in it.";
    }
    if (from == Kind::Device || to == Kind::Device) && selected.iter().any(|entry| entry.is_dir) {
        return "Folders cannot be copied to or from a phone yet. Select files only.";
    }
    if route.same_folder {
        return "Both sides show the same folder of this host. Open another folder on one side.";
    }
    ""
}

/// One thing to copy: its name and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Item {
    pub name: String,
    pub path: String,
}

/// A copy that passed every check: what goes where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Batch {
    pub from: Source,
    pub from_dir: String,
    pub to: Source,
    pub to_dir: String,
    pub items: Vec<Item>,
}

/// The job that copies one item of a batch, or None for a pair of places that has none.
pub(super) fn job_spec(batch: &Batch, item: &Item, serial: &str) -> Option<JobSpec> {
    let (path, to_dir) = (item.path.clone(), batch.to_dir.clone());
    Some(match (&batch.from, &batch.to) {
        (Source::Local, Source::Host(host)) => JobSpec::SftpUpload {
            host_id: host.clone(),
            local_path: path,
            remote_dir: to_dir,
            overwrite: false,
        },
        (Source::Host(host), Source::Local) => JobSpec::SftpDownload {
            host_id: host.clone(),
            remote_path: path,
            local_dir: to_dir,
            overwrite: false,
        },
        (Source::Local, Source::Device(route)) => JobSpec::DevicePush {
            route_id: route.clone(),
            serial: serial.to_string(),
            local_path: path,
            remote_dir: to_dir,
            overwrite: false,
        },
        (Source::Device(route), Source::Local) => JobSpec::DevicePull {
            route_id: route.clone(),
            remote_path: path,
            local_dir: to_dir,
            overwrite: false,
        },
        (Source::Host(host), Source::Device(route)) => JobSpec::HostToDevice {
            host_id: host.clone(),
            remote_path: path,
            route_id: route.clone(),
            serial: serial.to_string(),
            device_dir: to_dir,
            overwrite: false,
        },
        (Source::Device(route), Source::Host(host)) => JobSpec::DeviceToHost {
            route_id: route.clone(),
            device_path: path,
            host_id: host.clone(),
            remote_dir: to_dir,
            overwrite: false,
        },
        (Source::Host(from), Source::Host(to)) => JobSpec::HostToHost {
            from_host: from.clone(),
            remote_path: path,
            to_host: to.clone(),
            remote_dir: to_dir,
            overwrite: false,
        },
        _ => return None,
    })
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

    /// Copy the selected rows of `side` to the folder on the other side.
    pub(super) fn files_copy(self: &Rc<Self>, id: i32, side: usize) {
        if side > 1 {
            return;
        }
        // The button is only enabled when this works; a problem needs no words here.
        if let Ok(batch) = self.files_plan((id, side), (id, 1 - side), None) {
            self.files_send(batch);
        }
    }

    /// Check a copy of the selected rows of one side (`from`: pane id, side) into the folder
    /// of another (`to`), or into the folder on row `into` of that list. Gives the reason
    /// when it cannot be done.
    pub(super) fn files_plan(
        &self,
        from: (i32, usize),
        to: (i32, usize),
        into: Option<usize>,
    ) -> Result<Batch, &'static str> {
        const GONE: &str = "This list is no longer open.";
        let source = self
            .with_files(from.0, |pane| {
                let state = pane.sides.get(from.1)?.borrow();
                let selected: Vec<Entry> = state
                    .selected
                    .iter()
                    .filter_map(|row| state.entries.get(*row))
                    .cloned()
                    .collect();
                Some((state.source.clone(), state.path.clone(), selected))
            })
            .flatten();
        let Some((from_source, from_dir, selected)) = source else {
            return Err(GONE);
        };
        let target = self
            .with_files(to.0, |pane| {
                let state = pane.sides.get(to.1)?.borrow();
                let folder = match into {
                    Some(row) => {
                        let entry = state.entries.get(row).filter(|e| e.holds_files())?;
                        // A folder is not copied into itself.
                        if from == to && state.selected.contains(&row) {
                            return None;
                        }
                        join(state.source.kind(), &state.path, &entry.name)
                    }
                    None => state.path.clone(),
                };
                Some((state.source.clone(), folder, state.loaded))
            })
            .flatten();
        let Some((to_source, to_dir, to_loaded)) = target else {
            return Err("Files cannot be dropped here.");
        };
        let chosen: Vec<&Entry> = selected.iter().collect();
        let problem = copy_problem(
            Route {
                from: from_source.kind(),
                to: to_source.kind(),
                to_loaded,
                to_path: &to_dir,
                phone_ready: self.phone_ready(&from_source) && self.phone_ready(&to_source),
                same_folder: from_source == to_source
                    && same_dir(from_source.kind(), &from_dir, &to_dir),
            },
            &chosen,
        );
        if !problem.is_empty() {
            return Err(problem);
        }
        let items = selected
            .iter()
            .map(|entry| Item {
                name: entry.name.clone(),
                path: join(from_source.kind(), &from_dir, &entry.name),
            })
            .collect();
        Ok(Batch {
            from: from_source,
            from_dir,
            to: to_source,
            to_dir,
            items,
        })
    }

    /// Start one job for each item of a batch. A copy to or from a phone is confirmed first,
    /// naming the phone, its route and both folders.
    pub(super) fn files_send(self: &Rc<Self>, batch: Batch) {
        let (from_name, to_name) = (self.place_name(&batch.from), self.place_name(&batch.to));
        let phone = match (&batch.from, &batch.to) {
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
        for item in &batch.items {
            let Some(spec) = job_spec(&batch, item, &serial) else {
                return;
            };
            jobs.push((copy_label(&item.name, &from_name, &to_name), spec));
        }

        let Some(device) = phone else {
            for (label, spec) in jobs {
                self.job_start(label, spec);
            }
            return;
        };
        let names: Vec<&str> = batch.items.iter().map(|item| item.name.as_str()).collect();
        let mut facts = target_facts(&device);
        let files = if names.len() == 1 {
            names[0].to_string()
        } else {
            format!("{} files: {}", names.len(), names.join(", "))
        };
        facts.push(fact(
            if names.len() == 1 { "File" } else { "Files" },
            files,
            true,
        ));
        facts.push(fact(
            "From",
            format!("{from_name}: {}", batch.from_dir),
            true,
        ));
        facts.push(fact("To", format!("{to_name}: {}", batch.to_dir), true));
        let to_phone = matches!(batch.to, Source::Device(_));
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
