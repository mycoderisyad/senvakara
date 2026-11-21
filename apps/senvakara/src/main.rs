//! Senvakara desktop app, native Slint UI (ADR-023).
//!
//! Home (the inventory and its sections) plus session tabs. A tab holds one session, a
//! workspace of up to four sessions, or a run. Every session view takes its data as a struct
//! and names its session in callbacks, so the same view can be on screen more than once.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod automation;
mod desktop;
mod device;
mod files;
mod hostform;
mod inventory;
mod jobs;
mod logcat;
mod mirror;
mod palette;
mod run;
mod settings;
mod term;
mod wireless;
mod workspace;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use senvakara_adapters_pty::ShellKind;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

thread_local! {
    /// The app state, reachable from refresh requests queued by session threads.
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// The app state, when called on the UI thread after startup.
fn app() -> Option<Rc<App>> {
    APP.with(|app| app.borrow().clone())
}

/// A callback for session threads: asks the UI thread to redraw session `id`.
fn notifier(id: i32) -> impl Fn() + Send + Sync + 'static {
    move || {
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app() {
                app.refresh(id);
            }
        });
    }
}

/// What a session tab holds. The first five are single sessions and can also sit in a panel
/// of a workspace.
// A handful of long-lived panes: boxing the large ones would only add a hop to every access.
#[allow(clippy::large_enum_variant)]
pub enum Pane {
    Term(term::Session),
    Mirror(mirror::MirrorPane),
    Logcat(logcat::LogcatPane),
    /// Two file lists side by side: this PC, a host or a phone on each.
    Files(files::FilesPane),
    /// The screen of a host's X11 session.
    Desktop(desktop::DesktopPane),
    /// Up to four sessions side by side.
    Workspace(workspace::WorkspacePane),
    /// A Flutter build-and-deploy or a recipe run.
    Run(run::RunPane),
}

impl Pane {
    pub fn id(&self) -> i32 {
        match self {
            Pane::Term(session) => session.id,
            Pane::Mirror(pane) => pane.id,
            Pane::Logcat(pane) => pane.id,
            Pane::Files(pane) => pane.id,
            Pane::Desktop(pane) => pane.id,
            Pane::Workspace(pane) => pane.id,
            Pane::Run(pane) => pane.id,
        }
    }

    /// The view that draws it.
    fn kind(&self) -> &'static str {
        match self {
            Pane::Term(_) => "terminal",
            Pane::Mirror(_) => "mirror",
            Pane::Logcat(_) => "logcat",
            Pane::Files(_) => "files",
            Pane::Desktop(_) => "desktop",
            Pane::Workspace(_) => "workspace",
            Pane::Run(_) => "run",
        }
    }

    /// Whether closing it would end something that is still going.
    pub fn running(&self) -> bool {
        match self {
            Pane::Term(session) => !session.exited(),
            Pane::Mirror(pane) => !pane.ended.get(),
            Pane::Logcat(pane) => pane.active(),
            // Copies belong to the job list and go on without the tab.
            Pane::Files(_) => false,
            Pane::Desktop(pane) => !pane.ended(),
            Pane::Workspace(pane) => pane.running(),
            Pane::Run(pane) => pane.running(),
        }
    }

    /// What the confirmation says will happen when it is closed.
    pub fn closing_note(&self) -> String {
        match self {
            Pane::Term(_) => "The shell and everything running in it will end.".into(),
            Pane::Mirror(_) => "Screen mirroring of this phone will stop.".into(),
            Pane::Logcat(_) => {
                "The log stream stops and the lines collected here are discarded.".into()
            }
            Pane::Files(_) => "Copies that are running continue in the activity list.".into(),
            Pane::Desktop(_) => {
                "Screen sharing stops on the host. Programs running on its desktop are not affected.".into()
            }
            Pane::Workspace(pane) => pane.closing_note(),
            Pane::Run(_) => {
                "The run is cancelled: it stops at the next point where it can. What it already did on the phone stays.".into()
            }
        }
    }
}

/// Where a session is drawn: a tab of its own, or a panel (slot) of the workspace in a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Tab(usize),
    Slot(usize, usize),
}

impl Place {
    pub fn tab(self) -> usize {
        match self {
            Place::Tab(tab) | Place::Slot(tab, _) => tab,
        }
    }
}

/// What a confirmation dialog asks.
pub struct Question {
    pub title: String,
    pub body: String,
    pub confirm: String,
    pub danger: bool,
}

/// What happens when the user accepts the open confirmation.
pub enum Pending {
    CloseSession(i32),
    /// Host id and the fingerprint the user saw.
    TrustKey(String, String),
    /// A shell command for exactly the phone the dialog named.
    RunShell {
        route_id: String,
        serial: String,
        command: String,
    },
    /// Install a local APK; `label` is what the job list shows.
    InstallApk {
        label: String,
        route_id: String,
        serial: String,
        local_path: String,
    },
    Uninstall {
        route_id: String,
        serial: String,
        package: String,
    },
    /// Route id of a phone that is not connected.
    ForgetDevice(String),
    /// Route id of a Wi-Fi route.
    DisconnectWifi(String),
    /// Start a job again, this time replacing the file that was in the way. Label, job.
    ReplaceFile(String, jobs::JobSpec),
    /// Copies to or from a phone the dialog named: (label, job) each.
    CopyFiles(Vec<(String, jobs::JobSpec)>),
    /// Close the host form although the user typed in it, then go on to something else.
    DiscardHostForm(hostform::Next),
    /// Host id.
    RemoveHost(String),
    RemoveVpn {
        host_id: String,
        endpoint_id: String,
    },
    /// Database id of a saved workspace.
    DeleteWorkspace(String),
    /// Empty a panel whose session is still going. `workspace` is the pane id.
    CloseWorkspacePanel {
        workspace: i32,
        slot: usize,
    },
    /// Change to a layout that has no room for some panels.
    WorkspaceLayout {
        workspace: i32,
        layout: workspace::Layout,
    },
    /// Start the build-and-deploy the form describes.
    Deploy,
    /// Run the recipe open in the editor.
    RunRecipe,
    /// Database id of a recipe.
    DeleteRecipe(String),
}

struct App {
    /// The service layer: hosts, devices, sessions (`crates/app`).
    backend: Arc<senvakara_app::App>,
    inv: inventory::State,
    hosts: hostform::State,
    dev: device::State,
    jobs: jobs::State,
    wifi: wireless::State,
    ws: workspace::State,
    auto: automation::State,
    settings: settings::State,
    palette: palette::State,
    pending: RefCell<Option<Pending>>,
    ui: slint::Weak<MainWindow>,
    tabs: Rc<VecModel<TabInfo>>,
    sessions: RefCell<Vec<Pane>>,
    /// Index into `sessions`, or -1 for Home.
    active: Cell<i32>,
    next_id: Cell<i32>,
    /// The grid the last terminal view reported: the size a new terminal starts with.
    grid: Cell<(u16, u16)>,
    /// The terminal that last got a key or a click: copy and paste shortcuts go to it.
    focus_term: Cell<i32>,
}

impl App {
    /// The id for the next session.
    pub fn take_id(&self) -> i32 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    fn open_shell(self: &Rc<Self>, kind: &str, arg: &str) {
        let (shell, title) = match kind {
            "wsl" if arg.is_empty() => (ShellKind::Wsl { distro: None }, "WSL".to_string()),
            "wsl" => (
                ShellKind::Wsl {
                    distro: Some(arg.to_string()),
                },
                arg.to_string(),
            ),
            _ => (ShellKind::PowerShell, "PowerShell".to_string()),
        };
        let id = self.take_id();
        let (cols, lines) = self.grid.get();
        let notify = notifier(id);
        let Some(ui) = self.ui.upgrade() else { return };
        match term::Session::spawn(id, &shell, cols, lines, notify) {
            Ok(session) => {
                self.sessions.borrow_mut().push(Pane::Term(session));
                self.tabs.push(TabInfo {
                    id,
                    kind: "terminal".into(),
                    title: title.into(),
                    detail: SharedString::new(),
                    hue: if kind == "wsl" { 4 } else { 1 },
                    exited: false,
                });
                ui.global::<Inventory>().set_error(SharedString::new());
                self.select(self.tabs.row_count() as i32 - 1);
            }
            Err(error) => {
                ui.global::<Inventory>()
                    .set_error(format!("Could not start {title}: {error:#}").into());
                self.select(-1);
            }
        }
    }

    /// Where the session `id` is drawn.
    pub fn locate(&self, id: i32) -> Option<Place> {
        let sessions = self.sessions.borrow();
        sessions.iter().enumerate().find_map(|(tab, pane)| {
            if pane.id() == id {
                return Some(Place::Tab(tab));
            }
            match pane {
                Pane::Workspace(workspace) => {
                    workspace.slot_of(id).map(|slot| Place::Slot(tab, slot))
                }
                _ => None,
            }
        })
    }

    /// Run `f` on the pane at `place`. `f` must not open, close or move sessions.
    fn with_place<R>(&self, place: Place, f: impl FnOnce(&Pane) -> R) -> Option<R> {
        let sessions = self.sessions.borrow();
        match (place, sessions.get(place.tab())?) {
            (Place::Tab(_), pane) => Some(f(pane)),
            (Place::Slot(_, slot), Pane::Workspace(workspace)) => {
                let slots = workspace.slots.borrow();
                slots.get(slot)?.pane.as_ref().map(f)
            }
            _ => None,
        }
    }

    /// Run `f` on the session `id`, wherever it is drawn, with its place.
    pub fn with_pane<R>(&self, id: i32, f: impl FnOnce(&Pane, Place) -> R) -> Option<R> {
        let place = self.locate(id)?;
        self.with_place(place, |pane| f(pane, place))
    }

    /// Run `f` on every single session: the ones in tabs and the ones in workspace panels.
    pub fn each_pane(&self, mut f: impl FnMut(&Pane, Place)) {
        let sessions = self.sessions.borrow();
        for (tab, pane) in sessions.iter().enumerate() {
            match pane {
                Pane::Workspace(workspace) => {
                    for (slot, entry) in workspace.slots.borrow().iter().enumerate() {
                        if let Some(pane) = &entry.pane {
                            f(pane, Place::Slot(tab, slot));
                        }
                    }
                }
                pane => f(pane, Place::Tab(tab)),
            }
        }
    }

    /// Whether what is drawn at `place` is on screen.
    pub fn on_screen(&self, place: Place) -> bool {
        self.active.get() == place.tab() as i32
    }

    /// What the view at `place` currently shows.
    fn panel_load(&self, ui: &MainWindow, place: Place) -> PanelView {
        match place {
            Place::Tab(_) => ui.global::<Shell>().get_panel(),
            Place::Slot(tab, slot) => match self.sessions.borrow().get(tab) {
                Some(Pane::Workspace(workspace)) => workspace
                    .view
                    .row_data(slot)
                    .map(|row| row.panel)
                    .unwrap_or_default(),
                _ => PanelView::default(),
            },
        }
    }

    fn panel_store(&self, ui: &MainWindow, place: Place, panel: PanelView) {
        match place {
            Place::Tab(_) => ui.global::<Shell>().set_panel(panel),
            Place::Slot(tab, slot) => {
                if let Some(Pane::Workspace(workspace)) = self.sessions.borrow().get(tab) {
                    if let Some(mut row) = workspace.view.row_data(slot) {
                        row.panel = panel;
                        workspace.view.set_row_data(slot, row);
                    }
                }
            }
        }
    }

    /// Change what the view at `place` shows. `fresh` starts from an empty panel, dropping
    /// whatever another session left there.
    pub fn panel_update(
        &self,
        ui: &MainWindow,
        place: Place,
        fresh: bool,
        change: impl FnOnce(&mut PanelView),
    ) {
        let mut panel = if fresh {
            PanelView::default()
        } else {
            self.panel_load(ui, place)
        };
        change(&mut panel);
        self.panel_store(ui, place, panel);
    }

    /// Draw the session at `place` into its view. `full` redraws everything (the view was
    /// showing something else, or nothing).
    pub fn present_place(&self, ui: &MainWindow, place: Place, full: bool) {
        self.with_place(place, |pane| match pane {
            Pane::Term(session) => self.present(ui, session, place, full),
            Pane::Mirror(pane) => self.present_mirror(ui, pane, place, full),
            Pane::Logcat(pane) => self.present_logcat(ui, pane, place, full),
            Pane::Files(pane) => self.present_files(ui, pane, place, full),
            Pane::Desktop(pane) => self.present_desktop(ui, pane, place, full),
            Pane::Workspace(_) | Pane::Run(_) => {}
        });
    }

    /// A session was put in a tab or a panel: start what needs its place to exist.
    pub fn pane_started(self: &Rc<Self>, id: i32) {
        let files = self
            .with_pane(id, |pane, place| {
                // A desktop only sends its screen while it is on screen.
                if let Pane::Desktop(pane) = pane {
                    pane.set_visible(self.on_screen(place));
                }
                matches!(pane, Pane::Files(_))
            })
            .unwrap_or(false);
        if files {
            self.files_start(id);
        }
    }

    pub fn select(self: &Rc<Self>, index: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let count = self.sessions.borrow().len() as i32;
        let index = if index >= count {
            count - 1
        } else {
            index.max(-1)
        };
        self.active.set(index);
        // A desktop is only asked for its screen while its tab is the one on screen.
        self.each_pane(|pane, place| {
            if let Pane::Desktop(pane) = pane {
                pane.set_visible(place.tab() as i32 == index);
            }
        });
        let kind = (index >= 0)
            .then(|| self.sessions.borrow().get(index as usize).map(Pane::kind))
            .flatten()
            .unwrap_or("");
        match kind {
            "" => ui.global::<Shell>().set_panel(PanelView::default()),
            "workspace" => {
                ui.global::<Shell>().set_panel(PanelView::default());
                self.present_workspace(&ui, index as usize);
            }
            "run" => {
                ui.global::<Shell>().set_panel(PanelView::default());
                if let Some(Pane::Run(pane)) = self.sessions.borrow().get(index as usize) {
                    pane.take();
                    self.present_run(&ui, pane, true);
                }
            }
            _ => self.present_place(&ui, Place::Tab(index as usize), true),
        }
        ui.global::<Shell>().set_active_kind(kind.into());
        ui.global::<Shell>().set_active_tab(index);
    }

    pub fn close(self: &Rc<Self>, index: i32) {
        if index < 0 || index as usize >= self.sessions.borrow().len() {
            return;
        }
        // Dropping the pane ends its session: the PTY, the SSH channel or the mirror.
        let closed = self.sessions.borrow_mut().remove(index as usize);
        let workspace = matches!(closed, Pane::Workspace(_));
        drop(closed);
        self.tabs.remove(index as usize);
        let active = self.active.get();
        let next = if active == index {
            index.min(self.sessions.borrow().len() as i32 - 1)
        } else if active > index {
            active - 1
        } else {
            active
        };
        self.select(next);
        if workspace {
            self.workspaces_reload();
        }
    }

    /// Close a tab, asking first when its session is still running.
    fn request_close(self: &Rc<Self>, index: i32) {
        let sessions = self.sessions.borrow();
        let Some(pane) = (index >= 0).then(|| sessions.get(index as usize)).flatten() else {
            return;
        };
        if !pane.running() {
            drop(sessions);
            self.close(index);
            return;
        }
        let title = self
            .tabs
            .row_data(index as usize)
            .map(|tab| tab.title)
            .unwrap_or_default();
        let (id, body) = (pane.id(), pane.closing_note());
        let confirm = match pane {
            Pane::Workspace(_) => "Close workspace",
            Pane::Run(_) => "Cancel run and close",
            _ => "Close session",
        };
        drop(sessions);
        self.ask(
            Question {
                title: format!("Close {title}?"),
                body,
                confirm: confirm.into(),
                danger: true,
            },
            Pending::CloseSession(id),
        );
    }

    /// Show the confirmation dialog; `pending` runs if the user accepts.
    pub fn ask(&self, question: Question, pending: Pending) {
        self.ask_with(question, Vec::new(), "", pending);
    }

    /// Like [`App::ask`], listing exactly what will be affected. An empty confirm label makes
    /// the dialog informational: only Close is offered.
    pub fn ask_with(
        &self,
        question: Question,
        facts: Vec<KeyValue>,
        warning: &str,
        pending: Pending,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        let confirm = ui.global::<Confirm>();
        confirm.set_title(question.title.into());
        confirm.set_body(question.body.into());
        confirm.set_confirm_label(question.confirm.into());
        confirm.set_danger(question.danger);
        confirm.set_warning(warning.into());
        confirm.set_facts(ModelRc::new(VecModel::from(facts)));
        *self.pending.borrow_mut() = Some(pending);
        confirm.set_open(true);
    }

    fn answer(self: &Rc<Self>, accepted: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.global::<Confirm>().set_open(false);
        let Some(pending) = self.pending.borrow_mut().take() else {
            return;
        };
        if !accepted {
            return;
        }
        match pending {
            Pending::CloseSession(id) => {
                let index = self
                    .sessions
                    .borrow()
                    .iter()
                    .position(|pane| pane.id() == id);
                if let Some(index) = index {
                    self.close(index as i32);
                }
            }
            Pending::TrustKey(host, fingerprint) => self.trust_key(host, fingerprint),
            Pending::RunShell {
                route_id,
                serial,
                command,
            } => self.shell_run(route_id, serial, command),
            Pending::InstallApk {
                label,
                route_id,
                serial,
                local_path,
            } => self.job_start(
                label,
                jobs::JobSpec::InstallApk {
                    route_id,
                    serial,
                    local_path,
                },
            ),
            Pending::Uninstall {
                route_id,
                serial,
                package,
            } => self.uninstall(route_id, serial, package),
            Pending::ForgetDevice(route_id) => self.forget(route_id),
            Pending::DisconnectWifi(route_id) => self.disconnect_wifi(route_id),
            Pending::ReplaceFile(label, spec) => self.job_start(label, spec),
            Pending::CopyFiles(jobs) => {
                for (label, spec) in jobs {
                    self.job_start(label, spec);
                }
            }
            Pending::DiscardHostForm(next) => self.host_form_discard(next),
            Pending::RemoveHost(host_id) => self.host_remove(host_id),
            Pending::RemoveVpn {
                host_id,
                endpoint_id,
            } => self.paths_remove(host_id, endpoint_id),
            Pending::DeleteWorkspace(id) => self.workspace_delete(id),
            Pending::CloseWorkspacePanel { workspace, slot } => {
                self.workspace_clear(workspace, slot)
            }
            Pending::WorkspaceLayout { workspace, layout } => {
                self.workspace_set_layout(workspace, layout)
            }
            Pending::Deploy => self.deploy_start(),
            Pending::RunRecipe => self.recipe_run(),
            Pending::DeleteRecipe(id) => self.recipe_delete(id),
        }
    }

    /// Open a remote desktop tab on a connected host with an X11 session.
    fn open_desktop(self: &Rc<Self>, host: &senvakara_app::HostView) {
        if !host.connected {
            return;
        }
        let id = self.take_id();
        let hue = self.inv.hue(&host.id);
        let pane = desktop::DesktopPane::start(
            id,
            self.backend.clone(),
            host.id.clone(),
            host.display_name.clone(),
            hue,
            notifier(id),
        );
        self.sessions.borrow_mut().push(Pane::Desktop(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "desktop".into(),
            title: format!("{} desktop", host.display_name).into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }

    /// Draw a desktop pane: the newest picture, the state and why it ended.
    fn present_desktop(
        &self,
        ui: &MainWindow,
        pane: &desktop::DesktopPane,
        place: Place,
        full: bool,
    ) {
        let update = pane.take();
        // After a switch the view may still hold another desktop's picture.
        let frame = update
            .frame
            .or_else(|| full.then(|| pane.last.borrow().clone()).flatten());
        let phase = pane.phase.borrow();
        let (tone, text, error) = match &*phase {
            desktop::Phase::Starting => ("busy", "Starting", ""),
            desktop::Phase::Connected => ("live", "Connected", ""),
            desktop::Phase::Ended(reason) => ("idle", "Ended", reason.as_str()),
        };
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "desktop".into();
            let view = &mut panel.desktop;
            view.id = pane.id;
            view.title = pane.title.as_str().into();
            view.view_only = pane.view_only.get();
            if let Some(frame) = frame {
                view.frame_width = frame.width() as i32;
                view.frame_height = frame.height() as i32;
                view.frame = slint::Image::from_rgba8(frame);
            }
            view.tone = tone.into();
            view.state_text = text.into();
            view.error = error.into();
            view.live = *phase == desktop::Phase::Connected;
            view.detail = pane.detail.borrow().as_str().into();
        });
    }

    /// Draw a terminal into its view. The rows and the cursor are models of the session, so
    /// the view's data is only set when it starts showing this session (`full`).
    fn present(&self, ui: &MainWindow, session: &term::Session, place: Place, full: bool) {
        let cursor = session.sync(full);
        session.set_live(TermLive {
            cursor_col: cursor.col,
            cursor_row: cursor.row,
            cursor_visible: cursor.visible,
            exited: session.exited(),
            scrollback: cursor.scrollback,
        });
        if full {
            self.panel_update(ui, place, true, |panel| {
                panel.kind = "terminal".into();
                panel.term = TermData {
                    id: session.id,
                    rows: session.rows(),
                    live: session.live(),
                };
            });
        }
    }

    /// Draw a mirror pane: the newest frame, its state and any error.
    fn present_mirror(&self, ui: &MainWindow, pane: &mirror::MirrorPane, place: Place, full: bool) {
        let update = pane.take();
        // After a switch the view may hold another phone's frame: show this one's last.
        let frame = update
            .frame
            .or_else(|| full.then(|| pane.last.borrow().clone()).flatten());
        let (width, _) = pane.size();
        let (tone, text) = if pane.ended.get() {
            ("idle", "Ended".to_string())
        } else if width == 0 {
            ("busy", "Starting".to_string())
        } else {
            ("live", format!("Live, {} fps", pane.fps.get()))
        };
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "mirror".into();
            let view = &mut panel.mirror;
            view.id = pane.id;
            view.title = pane.title.as_str().into();
            view.route = pane.route.as_str().into();
            view.hue = pane.hue;
            if let Some(frame) = frame {
                view.frame_width = frame.width() as i32;
                view.frame_height = frame.height() as i32;
                view.frame = slint::Image::from_rgba8(frame);
            }
            view.tone = tone.into();
            view.state_text = text.into();
            view.live = !pane.ended.get();
            view.error = pane.error.borrow().as_str().into();
        });
    }

    /// Draw a logcat pane: its lines are a model of their own, so only the words around them
    /// are set here. A new `revision` tells the view to scroll to the end.
    fn present_logcat(&self, ui: &MainWindow, pane: &logcat::LogcatPane, place: Place, full: bool) {
        let changed = pane.take();
        if (changed || full) && pane.follow.get() {
            pane.revision.set(pane.revision.get().wrapping_add(1));
        }
        let (tone, text) = pane.status();
        self.panel_update(ui, place, full, |panel| {
            panel.kind = "logcat".into();
            panel.logcat = LogcatData {
                id: pane.id,
                lines: pane.model.clone().into(),
                title: pane.title.as_str().into(),
                route: pane.route.as_str().into(),
                hue: pane.hue,
                tone: tone.into(),
                state_text: text.into(),
                max_chars: pane.model.max_chars() as i32,
                empty_text: pane.empty_text().into(),
                active: pane.active(),
                error: pane.error.borrow().as_str().into(),
                filter: pane.filter.borrow().as_str().into(),
                follow: pane.follow.get(),
                revision: pane.revision.get(),
            };
        });
    }

    /// A session has new output, a new title or has ended.
    fn refresh(self: &Rc<Self>, id: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut title = None;
        let mut run_ended = false;
        let found = self.with_pane(id, |pane, place| {
            let visible = self.on_screen(place);
            match pane {
                Pane::Term(session) => {
                    title = session
                        .take_title()
                        .filter(|title| !title.trim().is_empty())
                        .map(|title| short_title(&title));
                    if visible {
                        self.present(&ui, session, place, false);
                    } else {
                        // Not visible: skip the drawing work. Showing it redraws it in full.
                        session.acknowledge();
                    }
                }
                Pane::Mirror(pane) => {
                    if visible {
                        self.present_mirror(&ui, pane, place, false);
                    } else {
                        // Drop frames nobody sees, but keep the ended state.
                        let _ = pane.take();
                    }
                }
                Pane::Logcat(pane) => {
                    if visible {
                        self.present_logcat(&ui, pane, place, false);
                    } else {
                        // Keep collecting: the newest lines are there when it is shown again.
                        let _ = pane.take();
                    }
                }
                // A file pane redraws itself when a listing arrives.
                Pane::Files(_) | Pane::Workspace(_) => {}
                Pane::Desktop(pane) => {
                    if visible {
                        self.present_desktop(&ui, pane, place, false);
                    } else {
                        // Keep the state (connected, ended and why); showing it draws it.
                        let _ = pane.take();
                    }
                }
                Pane::Run(pane) => {
                    // The run goes on whether or not its tab is on screen.
                    run_ended = pane.take();
                    if visible {
                        self.present_run(&ui, pane, false);
                    }
                }
            }
            (place, !pane.running())
        });
        // Only a tab of its own carries the session's title and its "ended" mark.
        if let Some((Place::Tab(index), exited)) = found {
            if let Some(mut tab) = self.tabs.row_data(index) {
                if title.is_some() || tab.exited != exited {
                    if let Some(title) = title {
                        tab.title = title.into();
                    }
                    tab.exited = exited;
                    self.tabs.set_row_data(index, tab);
                }
            }
        }
        if run_ended {
            self.automations_reload();
        }
    }

    fn with_term<R>(&self, id: i32, f: impl FnOnce(&term::Session) -> R) -> Option<R> {
        self.with_pane(id, |pane, _| match pane {
            Pane::Term(session) => Some(f(session)),
            _ => None,
        })
        .flatten()
    }

    /// The terminal the copy and paste shortcuts act on: the one the user last used, if it is
    /// on screen, otherwise the terminal of the active tab.
    fn with_active<R>(&self, f: impl FnOnce(&term::Session) -> R) -> Option<R> {
        let index = self.active.get();
        if index < 0 {
            return None;
        }
        let focused = self.focus_term.get();
        let id = match self.locate(focused) {
            Some(place) if self.on_screen(place) => focused,
            _ => self.sessions.borrow().get(index as usize)?.id(),
        };
        self.with_term(id, f)
    }

    fn with_mirror(&self, id: i32, f: impl FnOnce(&mirror::MirrorPane)) {
        self.with_pane(id, |pane, _| {
            if let Pane::Mirror(pane) = pane {
                f(pane);
            }
        });
    }

    fn with_desktop(&self, id: i32, f: impl FnOnce(&desktop::DesktopPane)) {
        self.with_pane(id, |pane, _| {
            if let Pane::Desktop(pane) = pane {
                f(pane);
            }
        });
    }

    /// Run `f` on a logcat pane, then redraw its status line.
    fn with_logcat(&self, id: i32, f: impl FnOnce(&logcat::LogcatPane)) {
        let Some(ui) = self.ui.upgrade() else { return };
        let found = self.with_pane(id, |pane, place| {
            let Pane::Logcat(pane) = pane else {
                return None;
            };
            f(pane);
            if self.on_screen(place) {
                self.present_logcat(&ui, pane, place, false);
            }
            Some((place, !pane.active()))
        });
        if let Some(Some((Place::Tab(index), exited))) = found {
            if let Some(mut tab) = self.tabs.row_data(index) {
                if tab.exited != exited {
                    tab.exited = exited;
                    self.tabs.set_row_data(index, tab);
                }
            }
        }
    }

    /// The user went to a section of Home: read what it shows.
    pub fn section_opened(self: &Rc<Self>, section: &str) {
        match section {
            "workspaces" => self.workspaces_reload(),
            "automations" => self.automations_opened(),
            "settings" => self.settings_opened(),
            _ => {}
        }
    }

    /// Window shortcuts: Ctrl+Tab, and Ctrl+Shift with T, W, C, V and K (the command palette).
    /// Plain Ctrl+T/W/K stay with the shell, as in Windows Terminal.
    fn shortcut(self: &Rc<Self>, text: &str, control: bool, shift: bool) -> bool {
        if !control {
            return false;
        }
        let key = text.chars().next().map(|c| c.to_ascii_lowercase());
        let tab =
            text.starts_with('\t') || text.starts_with(char::from(slint::platform::Key::Backtab));
        let count = self.sessions.borrow().len() as i32;
        if tab {
            // Home sits before the first session in the cycle.
            let step = if shift { -1 } else { 1 };
            let position = (self.active.get() + 1 + step).rem_euclid(count + 1);
            self.select(position - 1);
            return true;
        }
        if !shift {
            return false;
        }
        match key {
            Some('t') | Some('\u{14}') => {
                self.open_shell("powershell", "");
                true
            }
            Some('w') | Some('\u{17}') => {
                let active = self.active.get();
                if active >= 0 {
                    self.request_close(active);
                }
                true
            }
            Some('k') | Some('\u{b}') => {
                self.palette_toggle();
                true
            }
            Some('c') | Some('\u{3}') => {
                if let Some(text) = self
                    .with_active(|session| session.selection_text())
                    .flatten()
                {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
                true
            }
            Some('v') | Some('\u{16}') => {
                let text = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
                if let Ok(text) = text {
                    self.with_active(|session| session.paste(&text));
                }
                true
            }
            _ => false,
        }
    }
}

/// Windows shells title the window with the full path of their executable
/// ("C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe"); show just the program.
/// Any other title (a prompt such as "user@host: ~/project") is kept as the program set it.
fn short_title(title: &str) -> String {
    let title = title.trim();
    match title.strip_suffix(".exe") {
        Some(path) => path.rsplit(['\\', '/']).next().unwrap_or(path).to_string(),
        None => title.to_string(),
    }
}

/// Installed WSL distributions, default first. Empty when WSL is not installed.
/// Temporary: moves to `crates/app` with the rest of the backend (UI-MIG-06).
fn wsl_distros() -> Vec<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(output) = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    // wsl.exe prints UTF-16LE.
    let units: Vec<u16> = output
        .stdout
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn main() -> anyhow::Result<()> {
    let paths = senvakara_app::AppPaths::resolve()?;
    // One instance per data folder: a second one (this app or the old Tauri one) would reset
    // device states under the first.
    let _instance = match senvakara_app::single_instance(&paths) {
        Ok(guard) => Some(guard),
        Err(senvakara_app::InstanceError::AlreadyRunning) => {
            eprintln!("Senvakara is already running.");
            return Ok(());
        }
        Err(_) => None,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let backend = senvakara_app::App::start(paths, runtime.handle().clone())?;

    let ui = MainWindow::new()?;
    let app = Rc::new(App {
        backend,
        inv: inventory::State::default(),
        hosts: hostform::State::default(),
        dev: device::State::default(),
        jobs: jobs::State::default(),
        wifi: wireless::State::default(),
        ws: workspace::State::default(),
        auto: automation::State::default(),
        settings: settings::State::default(),
        palette: palette::State::default(),
        ui: ui.as_weak(),
        tabs: Rc::new(VecModel::default()),
        sessions: RefCell::new(Vec::new()),
        active: Cell::new(-1),
        next_id: Cell::new(1),
        grid: Cell::new((100, 30)),
        focus_term: Cell::new(0),
        pending: RefCell::new(None),
    });
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));

    let shell = ui.global::<Shell>();
    shell.set_tabs(app.tabs.clone().into());
    shell.on_new_session({
        let app = app.clone();
        move || app.open_shell("powershell", "")
    });

    app.init_inventory(&ui);
    app.init_device_tools(&ui);
    app.init_jobs(&ui);
    app.init_wireless(&ui);
    app.init_host_form(&ui);
    app.init_files(&ui);
    app.init_workspaces(&ui);
    app.init_automations(&ui);
    app.init_run(&ui);
    app.init_settings(&ui);
    app.init_palette(&ui);
    ui.global::<Nav>().on_opened({
        let app = app.clone();
        move |section| app.section_opened(&section)
    });

    ui.global::<Confirm>().on_answered({
        let app = app.clone();
        move |accepted| app.answer(accepted)
    });
    shell.on_select_tab({
        let app = app.clone();
        move |index| app.select(index)
    });
    shell.on_close_tab({
        let app = app.clone();
        move |index| app.request_close(index)
    });
    shell.on_shortcut({
        let app = app.clone();
        move |text, control, shift| app.shortcut(&text, control, shift)
    });

    let view = ui.global::<Term>();
    view.on_key({
        let app = app.clone();
        move |id, text, control, alt, shift| {
            app.focus_term.set(id);
            app.with_term(id, |session| session.key(&text, control, alt, shift))
                .unwrap_or(false)
        }
    });
    view.on_resized({
        let app = app.clone();
        move |id, cols, lines| {
            let grid = (cols.clamp(2, 1000) as u16, lines.clamp(1, 1000) as u16);
            // A tab's terminal sets the size new terminals start with; a small workspace
            // panel does not.
            if matches!(app.locate(id), Some(Place::Tab(_))) {
                app.grid.set(grid);
            }
            app.with_term(id, |session| session.resize(grid.0, grid.1));
        }
    });
    view.on_scrolled({
        let app = app.clone();
        move |id, lines, col, row, shift, control, alt| {
            let modifiers = term::Modifiers {
                shift,
                alt,
                control,
            };
            app.with_term(id, |session| {
                session.scroll(lines, col.max(0) as usize, row.max(0) as usize, modifiers)
            });
        }
    });
    view.on_pointer({
        let app = app.clone();
        move |id, kind, button, col, row, right_half, shift, control, alt| {
            let event = match kind {
                0 => term::Pointer::Down,
                1 => term::Pointer::Up,
                2 => term::Pointer::Move,
                _ => term::Pointer::Word,
            };
            if event == term::Pointer::Down {
                app.focus_term.set(id);
            }
            let modifiers = term::Modifiers {
                shift,
                alt,
                control,
            };
            app.with_term(id, |session| {
                session.pointer(
                    event,
                    button.clamp(0, 2) as u8,
                    col.max(0) as usize,
                    row.max(0) as usize,
                    right_half,
                    modifiers,
                )
            });
        }
    });
    let screen = ui.global::<Mirror>();
    screen.on_touch({
        let app = app.clone();
        move |id, action, x, y| {
            app.with_mirror(id, |pane| pane.touch(action.clamp(0, 2) as u8, x, y))
        }
    });
    screen.on_wheel({
        let app = app.clone();
        move |id, x, y, up| app.with_mirror(id, |pane| pane.wheel(x, y, up))
    });
    screen.on_key({
        let app = app.clone();
        move |id, text, control, alt| {
            let mut handled = false;
            app.with_mirror(id, |pane| handled = pane.key(&text, control, alt));
            handled
        }
    });
    screen.on_tool({
        let app = app.clone();
        move |id, tool| app.with_mirror(id, |pane| pane.tool(&tool))
    });
    let remote = ui.global::<Desktop>();
    remote.on_pointer({
        let app = app.clone();
        move |id, kind, button, x, y| app.with_desktop(id, |pane| pane.pointer(kind, button, x, y))
    });
    remote.on_wheel({
        let app = app.clone();
        move |id, x, y, up| app.with_desktop(id, |pane| pane.wheel(x, y, up))
    });
    remote.on_key({
        let app = app.clone();
        move |id, text, down| {
            let mut handled = false;
            app.with_desktop(id, |pane| handled = pane.key(&text, down));
            handled
        }
    });
    remote.on_blur({
        let app = app.clone();
        move |id| app.with_desktop(id, |pane| pane.release_all())
    });
    remote.on_set_view_only({
        let app = app.clone();
        move |id, on| {
            app.with_desktop(id, |pane| pane.set_view_only(on));
            // The box follows at once; the picture and state are redrawn as they are.
            app.refresh(id);
        }
    });
    remote.on_ctrl_alt_del({
        let app = app.clone();
        move |id| app.with_desktop(id, |pane| pane.send_ctrl_alt_del())
    });
    let log = ui.global::<Logcat>();
    log.on_toggle({
        let app = app.clone();
        move |id| {
            app.with_logcat(id, |pane| {
                if pane.active() {
                    pane.stop();
                } else {
                    pane.resume();
                }
            })
        }
    });
    log.on_clear({
        let app = app.clone();
        move |id| app.with_logcat(id, |pane| pane.clear())
    });
    log.on_filter_edited({
        let app = app.clone();
        move |id, text| app.with_logcat(id, |pane| pane.set_filter(&text))
    });
    log.on_follow_changed({
        let app = app.clone();
        move |id, follow| {
            app.with_logcat(id, |pane| {
                pane.follow.set(follow);
                if follow {
                    // Jump to the newest line.
                    pane.revision.set(pane.revision.get().wrapping_add(1));
                }
            })
        }
    });

    // Benchmarks and UI tests need a known window size: SENVAKARA_WINDOW_SIZE=936x776.
    if let Some((width, height)) = std::env::var("SENVAKARA_WINDOW_SIZE")
        .ok()
        .and_then(|size| {
            size.split_once('x')
                .map(|(w, h)| (w.parse::<f32>(), h.parse::<f32>()))
        })
        .and_then(|(w, h)| Some((w.ok()?, h.ok()?)))
    {
        ui.window().set_size(slint::LogicalSize::new(width, height));
    }

    ui.run()?;

    // Close every session before the UI goes away so no shell is left behind.
    app.sessions.borrow_mut().clear();
    // A desktop session stops its x11vnc on the host before the app goes away.
    desktop::wait_closed(std::time::Duration::from_secs(4));
    APP.with(|slot| slot.borrow_mut().take());
    Ok(())
}
