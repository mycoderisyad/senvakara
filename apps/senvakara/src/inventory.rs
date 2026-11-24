//! Home: hosts and devices from the backend, turned into the view models the UI draws, and the
//! actions the user can take on them.

mod device_actions;
mod devices;
mod groups;
mod host_actions;
mod hosts;
mod metrics;
mod quick_connect;
mod refresh;
mod selection;
mod text;

pub use hosts::{desktop_problem, host_subtitle};
pub use quick_connect::SshTarget;
pub use text::{fact, friendly, friendly_with, transport_label};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use senvakara_app::{HostView, Metrics};
use slint::VecModel;

use crate::{App, DeviceItem, HostGroup, HostItem};

type DeviceView = senvakara_app::android::DeviceView;

/// Everything Home shows, kept so the lists can be rebuilt after any change.
#[derive(Default)]
pub struct State {
    hosts: RefCell<Vec<HostView>>,
    devices: RefCell<Vec<DeviceView>>,
    watching: RefCell<HashMap<String, (bool, String)>>,
    local_adb: RefCell<Option<String>>,
    metrics: RefCell<HashMap<String, Metrics>>,
    /// The last failure per host, already worded for the user.
    errors: RefCell<HashMap<String, String>>,
    /// Hosts with a connect or disconnect in flight.
    busy: RefCell<HashSet<String>>,
    wsl: RefCell<Vec<String>>,
    /// Colour index per host id: its position in the host list, so two hosts never share one
    /// until there are more than eight.
    hues: RefCell<HashMap<String, i32>>,
    query: RefCell<String>,
    host_model: Rc<VecModel<HostItem>>,
    /// The same cards as `host_model`, filed by parent group for the Hosts page.
    group_model: Rc<VecModel<HostGroup>>,
    device_model: Rc<VecModel<DeviceItem>>,
    metrics_timer: slint::Timer,
    primed: Cell<bool>,
}

impl State {
    /// The colour index of a host, shared by its card, its tabs and its phones' route chips.
    pub fn hue(&self, host_id: &str) -> i32 {
        self.hues.borrow().get(host_id).copied().unwrap_or(0)
    }
}

impl App {
    /// Run a backend call and handle its result back on the UI thread.
    pub fn run<T: Send + 'static>(
        self: &Rc<Self>,
        work: impl std::future::Future<Output = T> + Send + 'static,
        done: impl FnOnce(&Rc<App>, T) + Send + 'static,
    ) {
        self.backend.rt.spawn(async move {
            let output = work.await;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    done(&app, output);
                }
            });
        });
    }
}
