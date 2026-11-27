//! A run tab: one workflow run, followed step by step with its log. The run lives in the backend; this pane collects its events on any thread and applies
//! them on the UI thread, so it keeps going while another tab is on screen.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use senvakara_app::{FlowEvent, Sink, WorkflowRemoveRequest, WorkflowRunRequest};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::{App, Run, RunStep};

mod words;

pub use words::{friendly_raw, run_state, when_text};

/// The log keeps this many lines; older ones are dropped.
pub const LOG_LIMIT: usize = 2000;

/// What to start.
pub enum Job {
    /// A saved workflow, with the sudo password and phone serials the user confirmed.
    Workflow(WorkflowRunRequest),
    /// One node's remove script, after the user confirmed it.
    WorkflowRemove(WorkflowRemoveRequest),
}

enum Signal {
    Flow(FlowEvent),
    /// The backend accepted the run and named it.
    Started(String),
    /// The backend refused to start it.
    Refused(String),
}

/// A cancel that is asked for before the backend has named the run is kept until it does.
#[derive(Default)]
struct Cancel {
    run_id: Option<String>,
    requested: bool,
}

impl Cancel {
    /// The user wants the run stopped. Returns the run to stop now, if it has an id yet.
    fn request(&mut self) -> Option<String> {
        self.requested = true;
        self.run_id.clone()
    }

    /// The backend named the run. Returns it when a cancel was already asked for.
    fn started(&mut self, run_id: &str) -> Option<String> {
        self.run_id = Some(run_id.to_string());
        self.requested.then(|| run_id.to_string())
    }
}

struct Shared {
    signals: Mutex<Vec<Signal>>,
    /// Behind one lock, so a cancel and the start of the run cannot miss each other.
    cancel: Mutex<Cancel>,
    dirty: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn push(&self, signal: Signal) {
        self.signals.lock().unwrap().push(signal);
        if !self.dirty.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }
}

/// The word shown next to a step's shape.
pub fn state_word(state: &str) -> &'static str {
    match state {
        "pending" => "Waiting",
        "running" => "Running",
        "succeeded" => "Done",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        "skipped" => "Skipped",
        _ => "Unknown",
    }
}

/// Split a chunk of output into lines without their line ends.
pub fn log_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    // A chunk that ends in a line break does not add an empty line.
    if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// How many of the oldest lines go when `adding` lines join `have` lines.
pub fn overflow(have: usize, adding: usize, limit: usize) -> usize {
    (have + adding).saturating_sub(limit).min(have)
}

pub struct RunPane {
    pub id: i32,
    pub title: String,
    /// "2201117PG, through RAFGT over USB"
    pub target: String,
    pub hue: i32,
    backend: Arc<senvakara_app::App>,
    shared: Arc<Shared>,
    steps: Rc<VecModel<RunStep>>,
    log: Rc<VecModel<SharedString>>,
    max_chars: Cell<usize>,
    revision: Cell<i32>,
    /// The backend's id of the run, once it has started.
    run_id: RefCell<Option<String>>,
    /// (status, message) when the run is over.
    done: RefCell<Option<(String, String)>>,
    cancelling: Cell<bool>,
}

impl RunPane {
    /// Open the pane and start the run. `labels` names the planned steps, in order. `notify`
    /// must only queue a refresh on the UI thread.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: i32,
        backend: Arc<senvakara_app::App>,
        title: String,
        target: String,
        hue: i32,
        labels: Vec<String>,
        job: Job,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            signals: Mutex::new(Vec::new()),
            cancel: Mutex::new(Cancel::default()),
            dirty: AtomicBool::new(false),
            notify: Box::new(notify),
        });
        let steps: Vec<RunStep> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| RunStep {
                number: (index + 1).to_string().into(),
                label: label.into(),
                state: "pending".into(),
                state_text: state_word("pending").into(),
                detail: SharedString::new(),
            })
            .collect();
        let sink: Sink<FlowEvent> = {
            let shared = shared.clone();
            Arc::new(move |event| {
                shared.push(Signal::Flow(event));
                true
            })
        };
        backend.rt.spawn({
            let (backend, shared) = (backend.clone(), shared.clone());
            async move {
                let started = match job {
                    Job::Workflow(request) => backend.workflow_run(request, sink).await,
                    Job::WorkflowRemove(request) => backend.workflow_remove(request, sink).await,
                };
                shared.push(match started {
                    Ok(run_id) => {
                        // The tab may have been closed before the run had an id.
                        let early = shared.cancel.lock().unwrap().started(&run_id);
                        if let Some(run_id) = early {
                            backend.flow_cancel(&run_id);
                        }
                        Signal::Started(run_id)
                    }
                    Err(error) => Signal::Refused(crate::inventory::friendly(&error)),
                });
            }
        });
        Self {
            id,
            title,
            target,
            hue,
            backend,
            shared,
            steps: Rc::new(VecModel::from(steps)),
            log: Rc::new(VecModel::default()),
            max_chars: Cell::new(0),
            revision: Cell::new(0),
            run_id: RefCell::new(None),
            done: RefCell::new(None),
            cancelling: Cell::new(false),
        }
    }

    /// Whether the run is still going.
    pub fn running(&self) -> bool {
        self.done.borrow().is_none()
    }

    /// Ask the backend to stop the run. It ends at the next point where it can. A run that has
    /// no id yet is stopped as soon as it gets one.
    pub fn cancel(&self) {
        if !self.running() {
            return;
        }
        let started = self.shared.cancel.lock().unwrap().request();
        if let Some(run_id) = started {
            self.backend.flow_cancel(&run_id);
        }
        self.cancelling.set(true);
    }

    fn finish(&self, status: &str, message: String) {
        // Steps that never started will not start now.
        for index in 0..self.steps.row_count() {
            if let Some(mut step) = self.steps.row_data(index) {
                if step.state == "pending" {
                    step.state = "skipped".into();
                    step.state_text = state_word("skipped").into();
                    self.steps.set_row_data(index, step);
                }
            }
        }
        *self.done.borrow_mut() = Some((status.to_string(), message));
    }

    /// Apply what arrived since the last call. Returns true when the run ended in this call.
    /// UI thread only.
    pub fn take(&self) -> bool {
        self.shared.dirty.store(false, Ordering::Release);
        let signals = std::mem::take(&mut *self.shared.signals.lock().unwrap());
        let mut ended = false;
        let mut lines: Vec<SharedString> = Vec::new();
        for signal in signals {
            match signal {
                Signal::Started(run_id) => {
                    *self.run_id.borrow_mut() = Some(run_id);
                }
                Signal::Refused(reason) => {
                    self.finish("failed", reason);
                    ended = true;
                }
                Signal::Flow(FlowEvent::Step {
                    ordinal,
                    status,
                    detail,
                    ..
                }) => {
                    if let Some(mut step) = self.steps.row_data(ordinal) {
                        step.state_text = state_word(&status).into();
                        step.state = status.into();
                        step.detail = detail.map(|d| friendly_raw(&d)).unwrap_or_default().into();
                        self.steps.set_row_data(ordinal, step);
                    }
                }
                Signal::Flow(FlowEvent::Log { text }) => {
                    lines.extend(log_lines(&text).into_iter().map(SharedString::from));
                }
                Signal::Flow(FlowEvent::Done { status, message }) => {
                    self.finish(&status, friendly_raw(&message));
                    ended = true;
                }
            }
        }
        if !lines.is_empty() {
            if lines.len() > LOG_LIMIT {
                lines.drain(..lines.len() - LOG_LIMIT);
            }
            for _ in 0..overflow(self.log.row_count(), lines.len(), LOG_LIMIT) {
                self.log.remove(0);
            }
            let longest = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0);
            self.max_chars.set(self.max_chars.get().max(longest));
            self.log.extend(lines);
            self.revision.set(self.revision.get().wrapping_add(1));
        }
        ended
    }
}

impl Drop for RunPane {
    fn drop(&mut self) {
        // Closing the tab of a run that is still going stops it; the user was asked first.
        self.cancel();
    }
}

impl App {
    pub fn init_run(self: &Rc<Self>, ui: &crate::MainWindow) {
        ui.global::<Run>().on_cancel({
            let app = self.clone();
            move || {
                let Some(ui) = app.ui.upgrade() else { return };
                let index = app.active.get();
                let sessions = app.sessions.borrow();
                if let Some(crate::Pane::Run(pane)) =
                    (index >= 0).then(|| sessions.get(index as usize)).flatten()
                {
                    pane.cancel();
                    app.present_run(&ui, pane, false);
                }
            }
        });
    }

    /// Draw the run tab on screen.
    pub fn present_run(&self, ui: &crate::MainWindow, pane: &RunPane, switched: bool) {
        let view = ui.global::<Run>();
        if switched {
            view.set_title(pane.title.as_str().into());
            view.set_target(pane.target.as_str().into());
            view.set_hue(pane.hue);
            view.set_steps(pane.steps.clone().into());
            view.set_log(pane.log.clone().into());
        }
        view.set_max_chars(pane.max_chars.get() as i32);
        let done = pane.done.borrow();
        let (tone, word, kind, text) = match &*done {
            None if pane.cancelling.get() => ("busy", "Cancelling", "", String::new()),
            None if pane.run_id.borrow().is_none() => ("busy", "Starting", "", String::new()),
            None => ("busy", "Running", "", String::new()),
            Some((status, message)) => match status.as_str() {
                "succeeded" => ("live", "Succeeded", "success", message.clone()),
                "cancelled" => (
                    "idle",
                    "Cancelled",
                    "warning",
                    if message.is_empty() {
                        "The run was cancelled.".to_string()
                    } else {
                        format!("The run was cancelled. {message}")
                    },
                ),
                _ => ("attention", "Failed", "danger", message.clone()),
            },
        };
        view.set_tone(tone.into());
        view.set_state_text(word.into());
        view.set_done_kind(kind.into());
        view.set_done_text(text.into());
        view.set_cancel_reason(
            if done.is_some() {
                "The run has ended."
            } else if pane.cancelling.get() {
                "The run stops at the next point where it can."
            } else if pane.run_id.borrow().is_none() {
                "The run is starting."
            } else {
                ""
            }
            .into(),
        );
        // Scrolls the log to its end; also when the tab comes back on screen.
        view.set_revision(pane.revision.get().wrapping_add(switched as i32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_split_into_lines() {
        assert_eq!(log_lines("one"), ["one"]);
        assert_eq!(log_lines("one\r\n"), ["one"]);
        assert_eq!(log_lines("one\ntwo\n"), ["one", "two"]);
        assert_eq!(log_lines("one\n\ntwo"), ["one", "", "two"]);
        assert_eq!(log_lines(""), [""]);
    }

    #[test]
    fn the_log_drops_its_oldest_lines() {
        assert_eq!(overflow(10, 5, LOG_LIMIT), 0);
        assert_eq!(overflow(1999, 1, LOG_LIMIT), 0);
        assert_eq!(overflow(2000, 1, LOG_LIMIT), 1);
        assert_eq!(overflow(1990, 30, LOG_LIMIT), 20);
        assert_eq!(overflow(5, 2000, LOG_LIMIT), 5);
    }

    #[test]
    fn a_cancel_before_the_run_has_an_id_is_sent_when_it_gets_one() {
        let mut early = Cancel::default();
        assert_eq!(early.request(), None);
        assert_eq!(early.started("run_1").as_deref(), Some("run_1"));

        let mut late = Cancel::default();
        assert_eq!(late.started("run_2"), None);
        assert_eq!(late.request().as_deref(), Some("run_2"));
    }

    #[test]
    fn every_step_state_has_a_word() {
        for state in [
            "pending",
            "running",
            "succeeded",
            "failed",
            "cancelled",
            "skipped",
        ] {
            assert_ne!(state_word(state), "Unknown");
        }
    }
}
