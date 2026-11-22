//! The job list at work: `impl App` to start, cancel, apply transfer events and publish rows.

use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use senvakara_app::{CmdResult, CoreError, ErrorCode, Sink, TransferEvent, TransferState};
use slint::{ComponentHandle, Model};

use crate::inventory::{fact, friendly_with};
use crate::{App, JobItem, Jobs, Pending, Question};

use super::{Job, JobSpec, Phase};

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
