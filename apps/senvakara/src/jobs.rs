//! Transfers and installs in progress: one compact list on Home, with progress, Cancel and the
//! result. A job is described by a [`JobSpec`], so it can be started again with "replace the
//! existing file" after the user agrees.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use senvakara_app::{CmdResult, CoreError, ErrorCode, Sink, TransferEvent, TransferState};
use slint::{ComponentHandle, Model, VecModel};

use crate::files::Source;
use crate::inventory::{fact, friendly_with};
use crate::{App, JobItem, Jobs, Pending, Question};

/// What a job does, with everything needed to start it (again).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSpec {
    InstallApk {
        route_id: String,
        serial: String,
        local_path: String,
    },
    /// Copy a file from this computer to a phone. Started by the Files pane, like the rest.
    DevicePush {
        route_id: String,
        serial: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a phone to this computer.
    DevicePull {
        route_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
    },
    /// Copy a file from this computer to a host over SFTP.
    SftpUpload {
        host_id: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a host to this computer over SFTP.
    SftpDownload {
        host_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a host straight to a phone, without a copy on this computer.
    HostToDevice {
        host_id: String,
        remote_path: String,
        route_id: String,
        serial: String,
        device_dir: String,
        overwrite: bool,
    },
    /// Copy a file from a phone straight to a host.
    DeviceToHost {
        route_id: String,
        device_path: String,
        host_id: String,
        remote_dir: String,
        overwrite: bool,
    },
}

impl JobSpec {
    /// The same job, allowed to replace an existing file. None when the job never overwrites
    /// or already does.
    pub fn replacing(&self) -> Option<JobSpec> {
        let mut spec = self.clone();
        match &mut spec {
            JobSpec::DevicePush { overwrite, .. }
            | JobSpec::DevicePull { overwrite, .. }
            | JobSpec::SftpUpload { overwrite, .. }
            | JobSpec::SftpDownload { overwrite, .. }
            | JobSpec::HostToDevice { overwrite, .. }
            | JobSpec::DeviceToHost { overwrite, .. }
                if !*overwrite =>
            {
                *overwrite = true;
                Some(spec)
            }
            _ => None,
        }
    }

    /// (file name, where it already exists) for the "Replace existing file?" question.
    fn target(&self) -> (String, String) {
        match self {
            JobSpec::InstallApk { local_path, .. } => (file_name(local_path), String::new()),
            JobSpec::DevicePush {
                local_path,
                remote_dir,
                ..
            } => (file_name(local_path), remote_dir.clone()),
            JobSpec::DevicePull {
                remote_path,
                local_dir,
                ..
            } => (file_name(remote_path), local_dir.clone()),
            JobSpec::SftpUpload {
                local_path,
                remote_dir,
                ..
            } => (file_name(local_path), remote_dir.clone()),
            JobSpec::SftpDownload {
                remote_path,
                local_dir,
                ..
            } => (file_name(remote_path), local_dir.clone()),
            JobSpec::HostToDevice {
                remote_path,
                device_dir,
                ..
            } => (file_name(remote_path), device_dir.clone()),
            JobSpec::DeviceToHost {
                device_path,
                remote_dir,
                ..
            } => (file_name(device_path), remote_dir.clone()),
        }
    }

    /// The place and folder a copy writes to. None for a job that is not a copy.
    pub fn destination(&self) -> Option<(Source, String)> {
        match self {
            JobSpec::InstallApk { .. } => None,
            JobSpec::DevicePush {
                route_id,
                remote_dir,
                ..
            } => Some((Source::Device(route_id.clone()), remote_dir.clone())),
            JobSpec::HostToDevice {
                route_id,
                device_dir,
                ..
            } => Some((Source::Device(route_id.clone()), device_dir.clone())),
            JobSpec::DevicePull { local_dir, .. } | JobSpec::SftpDownload { local_dir, .. } => {
                Some((Source::Local, local_dir.clone()))
            }
            JobSpec::SftpUpload {
                host_id,
                remote_dir,
                ..
            }
            | JobSpec::DeviceToHost {
                host_id,
                remote_dir,
                ..
            } => Some((Source::Host(host_id.clone()), remote_dir.clone())),
        }
    }

    /// What a "conflict" from the backend means for this kind of job.
    fn conflict_wording(&self) -> &'static str {
        match self {
            JobSpec::InstallApk { .. } => "The phone did not accept the app.",
            _ => "A file with that name already exists.",
        }
    }
}

/// The last component of a Windows or device path.
pub fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Starting,
    Running,
    Done,
    Failed,
    Cancelled,
}

struct Job {
    id: i32,
    label: String,
    spec: JobSpec,
    /// The backend's transfer id, once the job has started.
    remote: Option<u32>,
    phase: Phase,
    done: u64,
    total: u64,
    /// The result path or sentence, or the error.
    detail: String,
    /// Cancel was pressed before the backend had given the job an id.
    cancel_wanted: bool,
}

/// Progress as the list shows it: (word or percent, fraction of the bar).
fn progress(phase: Phase, done: u64, total: u64) -> (String, f32) {
    let fraction = if total > 0 {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    match phase {
        Phase::Starting => ("Starting".into(), 0.0),
        Phase::Running if total > 0 && done >= total => ("Finishing".into(), 1.0),
        Phase::Running if total > 0 => (format!("{} %", (fraction * 100.0).round()), fraction),
        Phase::Running => ("Working".into(), 0.0),
        Phase::Done => ("Done".into(), 1.0),
        Phase::Failed => ("Failed".into(), fraction),
        Phase::Cancelled => ("Cancelled".into(), fraction),
    }
}

impl Job {
    fn item(&self) -> JobItem {
        let (state_text, fraction) = progress(self.phase, self.done, self.total);
        let failed = self.phase == Phase::Failed;
        JobItem {
            id: self.id,
            label: self.label.as_str().into(),
            state: match self.phase {
                Phase::Starting | Phase::Running => "running",
                Phase::Done => "done",
                Phase::Failed => "failed",
                Phase::Cancelled => "cancelled",
            }
            .into(),
            state_text: state_text.into(),
            fraction,
            detail: if failed { "" } else { self.detail.as_str() }.into(),
            error: if failed { self.detail.as_str() } else { "" }.into(),
        }
    }

    fn finished(&self) -> bool {
        !matches!(self.phase, Phase::Starting | Phase::Running)
    }
}

/// Events from transfer threads, newest per job, waiting for the UI thread.
#[derive(Default)]
struct Inbox {
    events: Mutex<HashMap<i32, TransferEvent>>,
    dirty: AtomicBool,
}

#[derive(Default)]
pub struct State {
    jobs: RefCell<Vec<Job>>,
    model: Rc<VecModel<JobItem>>,
    next: Cell<i32>,
    inbox: Arc<Inbox>,
}

impl App {
    pub fn init_jobs(self: &Rc<Self>, ui: &crate::MainWindow) {
        let jobs = ui.global::<Jobs>();
        jobs.set_items(self.jobs.model.clone().into());
        jobs.on_cancel({
            let app = self.clone();
            move |id| app.job_cancel(id)
        });
        jobs.on_dismiss({
            let app = self.clone();
            move |id| {
                app.jobs
                    .jobs
                    .borrow_mut()
                    .retain(|job| job.id != id || !job.finished());
                app.jobs_publish();
            }
        });
        jobs.on_clear_finished({
            let app = self.clone();
            move || {
                app.jobs.jobs.borrow_mut().retain(|job| !job.finished());
                app.jobs_publish();
            }
        });
    }

    fn jobs_publish(&self) {
        let jobs = self.jobs.jobs.borrow();
        let items: Vec<JobItem> = jobs.iter().map(Job::item).collect();
        let model = &self.jobs.model;
        if model.row_count() == items.len() {
            // Same rows: update in place so a progress tick does not rebuild the list.
            for (row, item) in items.into_iter().enumerate() {
                if model.row_data(row).as_ref() != Some(&item) {
                    model.set_row_data(row, item);
                }
            }
        } else {
            model.set_vec(items);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Jobs>()
                .set_has_finished(jobs.iter().any(Job::finished));
        }
    }

    /// Start a transfer or install and show it in the job list.
    pub fn job_start(self: &Rc<Self>, label: String, spec: JobSpec) {
        let id = self.jobs.next.get() + 1;
        self.jobs.next.set(id);
        self.jobs.jobs.borrow_mut().insert(
            0,
            Job {
                id,
                label,
                spec: spec.clone(),
                remote: None,
                phase: Phase::Starting,
                done: 0,
                total: 0,
                detail: String::new(),
                cancel_wanted: false,
            },
        );
        self.jobs_publish();

        let sink: Sink<TransferEvent> = {
            let inbox = self.jobs.inbox.clone();
            Arc::new(move |event| {
                inbox.events.lock().unwrap().insert(id, event);
                if !inbox.dirty.swap(true, Ordering::AcqRel) {
                    let _ = slint::invoke_from_event_loop(|| {
                        if let Some(app) = crate::app() {
                            app.jobs_drain();
                        }
                    });
                }
                true
            })
        };
        let backend = self.backend.clone();
        self.run(
            async move {
                let started: CmdResult<u32> = match spec {
                    JobSpec::InstallApk {
                        route_id,
                        serial,
                        local_path,
                    } => {
                        backend
                            .device_install_apk(route_id, serial, local_path, sink)
                            .await
                    }
                    JobSpec::DevicePush {
                        route_id,
                        serial,
                        local_path,
                        remote_dir,
                        overwrite,
                    } => {
                        backend
                            .device_push(route_id, serial, local_path, remote_dir, overwrite, sink)
                            .await
                    }
                    JobSpec::DevicePull {
                        route_id,
                        remote_path,
                        local_dir,
                        overwrite,
                    } => {
                        backend
                            .device_pull(route_id, remote_path, local_dir, overwrite, sink)
                            .await
                    }
                    JobSpec::SftpUpload {
                        host_id,
                        local_path,
                        remote_dir,
                        overwrite,
                    } => {
                        backend
                            .sftp_upload(host_id, local_path, remote_dir, overwrite, sink)
                            .await
                    }
                    JobSpec::SftpDownload {
                        host_id,
                        remote_path,
                        local_dir,
                        overwrite,
                    } => {
                        backend
                            .sftp_download(host_id, remote_path, local_dir, overwrite, sink)
                            .await
                    }
                    JobSpec::HostToDevice {
                        host_id,
                        remote_path,
                        route_id,
                        serial,
                        device_dir,
                        overwrite,
                    } => {
                        backend
                            .xfer_host_to_device(
                                host_id,
                                remote_path,
                                route_id,
                                serial,
                                device_dir,
                                overwrite,
                                sink,
                            )
                            .await
                    }
                    JobSpec::DeviceToHost {
                        route_id,
                        device_path,
                        host_id,
                        remote_dir,
                        overwrite,
                    } => {
                        backend
                            .xfer_device_to_host(
                                route_id,
                                device_path,
                                host_id,
                                remote_dir,
                                overwrite,
                                sink,
                            )
                            .await
                    }
                };
                started
            },
            move |app, started| match started {
                Ok(remote) => {
                    let mut cancel = false;
                    if let Some(job) = app.jobs.jobs.borrow_mut().iter_mut().find(|j| j.id == id) {
                        job.remote = Some(remote);
                        if job.phase == Phase::Starting {
                            job.phase = Phase::Running;
                        }
                        cancel = job.cancel_wanted && !job.finished();
                    }
                    if cancel {
                        app.backend.transfer_cancel(remote);
                    }
                    app.jobs_publish();
                }
                Err(error) => app.job_failed(id, error),
            },
        );
    }

    /// Apply the newest event of every job that reported since the last call.
    fn jobs_drain(self: &Rc<Self>) {
        self.jobs.inbox.dirty.store(false, Ordering::Release);
        let events = std::mem::take(&mut *self.jobs.inbox.events.lock().unwrap());
        let mut finished = Vec::new();
        let mut failed = Vec::new();
        for (id, event) in events {
            let mut jobs = self.jobs.jobs.borrow_mut();
            let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
                continue;
            };
            job.remote = Some(event.id);
            (job.done, job.total) = (event.done, event.total);
            match event.state {
                TransferState::Running => job.phase = Phase::Running,
                TransferState::Done => {
                    job.phase = Phase::Done;
                    job.detail = event.result_path.unwrap_or_default();
                    finished.push(job.spec.clone());
                }
                TransferState::Cancelled => job.phase = Phase::Cancelled,
                TransferState::Failed => {
                    failed.push((id, event.error.unwrap_or_else(|| CoreError::internal(""))))
                }
            }
        }
        for (id, error) in failed {
            self.job_failed(id, error);
        }
        self.jobs_publish();
        for spec in finished {
            self.job_finished(&spec);
        }
    }

    /// A job could not start or stopped with an error. A name clash is not a failure yet:
    /// ask whether to replace the file and start again.
    fn job_failed(self: &Rc<Self>, id: i32, error: CoreError) {
        let mut jobs = self.jobs.jobs.borrow_mut();
        let Some(index) = jobs.iter().position(|job| job.id == id) else {
            return;
        };
        let retry = (error.code == ErrorCode::Conflict)
            .then(|| jobs[index].spec.replacing())
            .flatten();
        if let Some(spec) = retry {
            let job = jobs.remove(index);
            drop(jobs);
            self.jobs_publish();
            let (name, place) = job.spec.target();
            self.ask_with(
                Question {
                    title: "Replace existing file?".into(),
                    body: "A file with this name is already there. Replacing it cannot be undone."
                        .into(),
                    confirm: "Replace".into(),
                    danger: true,
                },
                vec![fact("File", name, true), fact("Already in", place, true)],
                "",
                Pending::ReplaceFile(job.label, spec),
            );
            return;
        }
        let job = &mut jobs[index];
        if error.code == ErrorCode::Cancelled {
            job.phase = Phase::Cancelled;
        } else {
            job.phase = Phase::Failed;
            job.detail = friendly_with(&error, job.spec.conflict_wording());
        }
        drop(jobs);
        self.jobs_publish();
    }

    fn job_cancel(&self, id: i32) {
        let mut jobs = self.jobs.jobs.borrow_mut();
        let Some(job) = jobs.iter_mut().find(|job| job.id == id) else {
            return;
        };
        if job.finished() {
            return;
        }
        match job.remote {
            Some(remote) => self.backend.transfer_cancel(remote),
            None => job.cancel_wanted = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_words() {
        assert_eq!(progress(Phase::Starting, 0, 0), ("Starting".into(), 0.0));
        assert_eq!(progress(Phase::Running, 0, 0), ("Working".into(), 0.0));
        assert_eq!(progress(Phase::Running, 1, 4), ("25 %".into(), 0.25));
        // Upload finished, the phone is still installing.
        assert_eq!(progress(Phase::Running, 4, 4), ("Finishing".into(), 1.0));
        assert_eq!(progress(Phase::Done, 0, 0), ("Done".into(), 1.0));
        assert_eq!(progress(Phase::Cancelled, 1, 2), ("Cancelled".into(), 0.5));
        assert_eq!(progress(Phase::Failed, 9, 3).1, 1.0);
    }

    #[test]
    fn only_file_copies_can_be_retried_with_replace() {
        let install = JobSpec::InstallApk {
            route_id: "r".into(),
            serial: "s".into(),
            local_path: "C:\\apps\\demo.apk".into(),
        };
        assert_eq!(install.replacing(), None);

        let pull = JobSpec::DevicePull {
            route_id: "r".into(),
            remote_path: "/sdcard/Download/a.txt".into(),
            local_dir: "C:\\Users\\me".into(),
            overwrite: false,
        };
        let again = pull.replacing().expect("a pull can replace");
        assert!(matches!(
            again,
            JobSpec::DevicePull {
                overwrite: true,
                ..
            }
        ));
        // Already replacing: a second clash is a real failure.
        assert_eq!(again.replacing(), None);
        assert_eq!(
            pull.target(),
            ("a.txt".to_string(), "C:\\Users\\me".to_string())
        );
    }

    #[test]
    fn copies_know_where_they_write() {
        let upload = JobSpec::SftpUpload {
            host_id: "h".into(),
            local_path: "C:\\Temp\\a.txt".into(),
            remote_dir: "/tmp".into(),
            overwrite: false,
        };
        assert_eq!(
            upload.destination(),
            Some((Source::Host("h".into()), "/tmp".to_string()))
        );
        assert_eq!(upload.target(), ("a.txt".to_string(), "/tmp".to_string()));
        assert!(upload.replacing().is_some());
        let across = JobSpec::HostToDevice {
            host_id: "h".into(),
            remote_path: "/tmp/a.txt".into(),
            route_id: "r".into(),
            serial: "s".into(),
            device_dir: "/sdcard/Download".into(),
            overwrite: true,
        };
        assert_eq!(
            across.destination(),
            Some((Source::Device("r".into()), "/sdcard/Download".to_string()))
        );
        assert_eq!(across.replacing(), None);
        let install = JobSpec::InstallApk {
            route_id: "r".into(),
            serial: "s".into(),
            local_path: "C:\\apps\\demo.apk".into(),
        };
        assert_eq!(install.destination(), None);
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("C:\\apps\\demo.apk"), "demo.apk");
        assert_eq!(file_name("/sdcard/Download/a.txt"), "a.txt");
        assert_eq!(file_name("/sdcard/Download/"), "Download");
        assert_eq!(file_name("plain"), "plain");
    }
}
