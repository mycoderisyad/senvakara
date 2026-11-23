//! Transfers and installs in progress: one compact list on Home, with progress, Cancel and the
//! result. A job is described by a [`JobSpec`], so it can be started again with "replace the
//! existing file" after the user agrees.

mod queue;
mod spec;

pub use spec::{file_name, JobSpec};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use senvakara_app::TransferEvent;
use slint::VecModel;

use crate::JobItem;

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

/// Name clashes waiting for the user's answer, oldest first: (label, job that replaces).
/// The confirmation dialog shows one question at a time, so the rest wait here instead of
/// overwriting the open one.
#[derive(Default)]
struct Conflicts(VecDeque<(String, JobSpec)>);

impl Conflicts {
    fn push(&mut self, label: String, spec: JobSpec) {
        self.0.push_back((label, spec));
    }

    /// The next clash to ask about, unless a dialog is open: that one is answered first.
    fn take_next(&mut self, dialog_open: bool) -> Option<(String, JobSpec)> {
        if dialog_open {
            return None;
        }
        self.0.pop_front()
    }
}

#[derive(Default)]
pub struct State {
    jobs: RefCell<Vec<Job>>,
    model: Rc<VecModel<JobItem>>,
    next: Cell<i32>,
    inbox: Arc<Inbox>,
    conflicts: RefCell<Conflicts>,
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

    fn pull(name: &str) -> JobSpec {
        JobSpec::DevicePull {
            route_id: "r".into(),
            remote_path: format!("/sdcard/{name}"),
            local_dir: "/home/me".into(),
            overwrite: true,
        }
    }

    #[test]
    fn name_clashes_are_asked_one_at_a_time_in_order() {
        let mut waiting = Conflicts::default();
        waiting.push("first".into(), pull("a.txt"));
        waiting.push("second".into(), pull("b.txt"));
        // Another dialog is open: nothing is asked and nothing is lost.
        assert_eq!(waiting.take_next(true), None);
        assert_eq!(
            waiting.take_next(false),
            Some(("first".into(), pull("a.txt")))
        );
        assert_eq!(
            waiting.take_next(false),
            Some(("second".into(), pull("b.txt")))
        );
        assert_eq!(waiting.take_next(false), None);
    }
}
