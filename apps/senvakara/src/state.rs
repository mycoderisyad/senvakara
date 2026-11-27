//! The app state: `App`, the UI thread's handle to it and the redraw notifier for session threads.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use slint::VecModel;

use crate::{
    device, files, hostform, inventory, jobs, keychain, launcher, palette, platform, settings,
    wireless, workflows, workspace, MainWindow, Pane, Pending, TabInfo,
};

thread_local! {
    /// The app state, reachable from refresh requests queued by session threads.
    pub(crate) static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// The app state, when called on the UI thread after startup.
pub(crate) fn app() -> Option<Rc<App>> {
    APP.with(|app| app.borrow().clone())
}

/// A callback for session threads: asks the UI thread to redraw session `id`.
pub(crate) fn notifier(id: i32) -> impl Fn() + Send + Sync + 'static {
    move || {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app() {
                app.refresh(id);
            }
        });
    }
}

pub(crate) struct App {
    /// The service layer: hosts, devices, sessions (`crates/app`).
    pub(crate) backend: Arc<senvakara_app::App>,
    pub(crate) inv: inventory::State,
    pub(crate) hosts: hostform::State,
    pub(crate) keys: keychain::State,
    /// Files dragged in from Explorer (`platform::FileDrops`).
    pub(crate) drops: platform::FileDrops,
    pub(crate) files: files::State,
    pub(crate) dev: device::State,
    pub(crate) jobs: jobs::State,
    pub(crate) wifi: wireless::State,
    pub(crate) ws: workspace::State,
    pub(crate) wf: workflows::State,
    pub(crate) settings: settings::State,
    pub(crate) palette: palette::State,
    pub(crate) launcher: launcher::State,
    pub(crate) pending: RefCell<Option<Pending>>,
    /// Questions asked while the dialog was busy with another, oldest first.
    pub(crate) asks: RefCell<std::collections::VecDeque<crate::confirm::Asked>>,
    pub(crate) ui: slint::Weak<MainWindow>,
    pub(crate) tabs: Rc<VecModel<TabInfo>>,
    pub(crate) sessions: RefCell<Vec<Pane>>,
    /// Index into `sessions`, or -1 for Home.
    pub(crate) active: Cell<i32>,
    pub(crate) next_id: Cell<i32>,
    /// The grid the last terminal view reported: the size a new terminal starts with.
    pub(crate) grid: Cell<(u16, u16)>,
    /// The terminal that last got a key or a click: copy and paste shortcuts go to it.
    pub(crate) focus_term: Cell<i32>,
}

impl App {
    /// The id for the next session.
    pub fn take_id(&self) -> i32 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }
}
