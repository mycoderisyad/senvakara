//! Senvakara's headless service layer: everything the app does, with no UI
//! toolkit in sight. A front end creates one [`App`], calls its methods and
//! renders what comes back.
//!
//! Rules of the boundary:
//! - Every background job is spawned on [`App::rt`]; callers need no runtime
//!   context of their own. `async` methods run their work on that runtime
//!   too, so they can be awaited from any executor (a UI event loop
//!   included) and never need `block_on`. The few sync methods only touch
//!   memory or the local database.
//! - A session (terminal, mirror, logcat, transfer, run) reports to the
//!   [`Sink`] it was started with, and its stream carries its own end event.
//!   A sink returns `false` when its viewer is gone, which stops the session.
//! - [`App::events`] only carries refresh hints ([`AppEvent`]); a receiver
//!   that lagged reloads its lists.
//! - Sinks and event receivers only queue: they are never called while an
//!   internal lock is held, and they must not call back into `App`
//!   synchronously.

pub mod android;
pub mod android_tools;
pub mod crosshost;
pub mod desktop;
pub mod events;
pub mod files;
pub mod flow;
pub mod hostcopy;
pub mod hosts;
pub mod instance;
pub mod keychain;
pub mod logging;
pub mod mirror;
pub mod monitor;
pub mod paths;
pub mod rdp;
pub mod recipes;
pub mod shells;
pub mod sink;
pub mod state;
pub mod terminal;
pub mod wfrun;
pub mod workflows;
pub mod workspaces;

pub use android::{AdbStatus, DeviceInfo, DeviceView, ShellResult, WatchStatus};
pub use desktop::{DesktopInfo, VncTarget};
pub use events::{AppEvent, LogcatEvent, MirrorEvent, TermEvent};
pub use files::{TransferEvent, TransferState};
pub use flow::{FlowEvent, FlowSpec, FlutterInfo};
pub use hosts::{HostKeyCheck, HostView, RdpView, SshView};
pub use instance::{single_instance, InstanceError, InstanceGuard};
pub use keychain::{KeyCreate, KeyImport, KeyUpdate, KeyView};
pub use mirror::MirrorStarted;
pub use paths::AppPaths;
pub use shells::LocalShell;
pub use sink::Sink;
pub use state::{ActivePath, AppState, CmdResult};
pub use terminal::ShellRequest;
pub use wfrun::WorkflowRunRequest;
pub use workflows::{catalog, workflow_problems, WorkflowView};

// Types of the lower layers that appear in this crate's API.
pub use senvakara_adapters_adb::sync::SyncEntry;
pub use senvakara_adapters_adb::wireless::MdnsService;
pub use senvakara_adapters_scrcpy::control::Input;
pub use senvakara_adapters_ssh::keys::is_encrypted_private_key;
pub use senvakara_adapters_ssh::sftp::{Listing, RemoteEntry};
pub use senvakara_adapters_ssh::{Secret, SshConnection};
pub use senvakara_core::{CoreError, ErrorCode};
pub use senvakara_metrics::Metrics;
pub use senvakara_storage::{
    DeviceRouteRecord, HostUpsert, KeyKind, PanelRecord, Protocol, RecipePermissions, RecipeRecord,
    RecipeSave, RecipeStep, RunRecord, SshAuth, StepRecord, VpnEndpoint, WorkspaceRecord,
    WorkspaceSave,
};
/// The workflow graph model, for front ends that edit graphs.
pub use senvakara_workflow as workflow;
pub use senvakara_workflow::{Catalog, Problem as WorkflowProblem, Workflow};

use anyhow::Context;
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

/// Refresh hints are tiny and rare; a receiver this far behind just reloads.
const EVENT_BUFFER: usize = 256;

/// The running application: database, connections and every live session.
pub struct App {
    /// Runtime every background job runs on.
    pub rt: Handle,
    pub paths: AppPaths,
    /// Refresh hints; see [`App::subscribe`].
    pub events: broadcast::Sender<AppEvent>,
    /// Database, credential store and the SSH connection pool.
    pub state: AppState,
    pub android: android::Android,
    pub tools: android_tools::AndroidTools,
    pub terms: terminal::Terminals,
    pub transfers: files::Transfers,
    pub mirrors: mirror::Mirrors,
    pub monitor: monitor::Monitor,
    pub desktops: desktop::Desktops,
    pub flows: flow::Flows,
}

impl App {
    /// Open the database, start logging, start watching the local ADB server
    /// and start the connection heartbeat. Does not take the single-instance
    /// lock: call [`single_instance`] first.
    pub fn start(paths: AppPaths, rt: Handle) -> anyhow::Result<Arc<App>> {
        logging::init(paths.log_dir.clone());
        std::fs::create_dir_all(&paths.data_dir)
            .with_context(|| format!("create {}", paths.data_dir.display()))?;
        let state = AppState::open(paths.db_path.clone())
            .with_context(|| format!("open {}", paths.db_path.display()))?;
        logging::info("app started");
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let app = Arc::new(App {
            rt,
            paths,
            events,
            state,
            android: Default::default(),
            tools: Default::default(),
            terms: Default::default(),
            transfers: Default::default(),
            mirrors: Default::default(),
            monitor: Default::default(),
            desktops: Default::default(),
            flows: Default::default(),
        });
        workflows::convert_recipes(&app);
        android::start_local(&app);
        monitor::spawn_heartbeat(&app);
        // Whatever opens a host (a terminal, Files, a desktop), its phones are
        // watched while it is connected, not only after Connect.
        let weak = Arc::downgrade(&app);
        app.state.set_on_connect(move |host_id| {
            let Some(app) = weak.upgrade() else { return };
            let Some(conn) = app.state.live(host_id) else {
                return;
            };
            let host_id = host_id.to_string();
            let task_app = app.clone();
            app.rt.spawn(async move {
                hosts::prepare_host(&task_app, &host_id, conn, false).await;
            });
        });
        Ok(app)
    }

    /// Receive refresh hints. On `RecvError::Lagged`, reload everything.
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    /// Queue a refresh hint (nobody listening is fine).
    pub(crate) fn emit(&self, event: AppEvent) {
        let _ = self.events.send(event);
    }

    /// Run `work` on the app runtime and wait for it, from any executor.
    async fn run<T, F>(&self, work: F) -> CmdResult<T>
    where
        T: Send + 'static,
        F: Future<Output = CmdResult<T>> + Send + 'static,
    {
        match self.rt.spawn(work).await {
            Ok(result) => result,
            Err(e) => Err(CoreError::internal(format!("background task failed: {e}"))),
        }
    }
}

impl App {
    /// Run blocking `work` (credential store, key decryption, file reads) on
    /// the runtime's blocking pool and wait for it, from any executor.
    async fn blocking<T, F>(&self, work: F) -> CmdResult<T>
    where
        T: Send + 'static,
        F: FnOnce() -> CmdResult<T> + Send + 'static,
    {
        match self.rt.spawn_blocking(work).await {
            Ok(result) => result,
            Err(e) => Err(CoreError::internal(format!("background task failed: {e}"))),
        }
    }
}

/// `self.run(...)` over a module function that takes `&Arc<App>` first.
macro_rules! on_rt {
    ($self:ident, $($path:ident)::+ ( $($arg:expr),* )) => {{
        let app = $self.clone();
        $self.run(async move { $($path)::+(&app $(, $arg)*).await }).await
    }};
}

// ------------------------------------------------------------------- hosts
impl App {
    pub fn hosts_list(&self) -> CmdResult<Vec<HostView>> {
        hosts::hosts_list(self)
    }

    pub fn host_create(&self, input: HostUpsert) -> CmdResult<HostView> {
        hosts::host_create(self, input)
    }

    /// Saves the settings and drops the host's connection and device watch.
    pub async fn host_update(
        self: &Arc<Self>,
        id: String,
        input: HostUpsert,
    ) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_update(id, input))
    }

    pub async fn host_delete(self: &Arc<Self>, id: String) -> CmdResult<()> {
        on_rt!(self, hosts::host_delete(id))
    }

    /// Read the host key the server presents, without authenticating.
    pub async fn host_check_key(self: &Arc<Self>, id: String) -> CmdResult<HostKeyCheck> {
        on_rt!(self, hosts::host_check_key(id))
    }

    /// Pin the fingerprint the user approved (the server is asked again).
    pub async fn host_trust_key(
        self: &Arc<Self>,
        id: String,
        fingerprint: String,
    ) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_trust_key(id, fingerprint))
    }

    /// Save (`Some`) or clear (`None` / empty) the private-key passphrase.
    pub async fn host_set_passphrase(
        self: &Arc<Self>,
        id: String,
        passphrase: Option<String>,
    ) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_set_passphrase(id, passphrase))
    }

    /// Save (`Some`) or clear (`None` / empty) the SSH login password, then
    /// drop the host's connection so the next one signs in with it.
    pub async fn host_set_password(
        self: &Arc<Self>,
        id: String,
        password: Option<String>,
    ) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_set_password(id, password))
    }

    /// Save (`Some`) or clear (`None` / empty) the password Remote Desktop
    /// signs in with, under `TERMSRV/<address>` in the OS store.
    pub async fn host_set_rdp_password(
        self: &Arc<Self>,
        id: String,
        password: Option<String>,
    ) -> CmdResult<HostView> {
        let app = self.clone();
        self.blocking(move || hosts::host_set_rdp_password(&app.state, &id, password))
            .await
    }

    /// Open a Remote Desktop host in `mstsc.exe`; returns once it started.
    pub async fn rdp_launch(self: &Arc<Self>, host_id: String) -> CmdResult<()> {
        on_rt!(self, rdp::rdp_launch(host_id))
    }

    /// Connect (or reuse the connection), refresh capabilities, watch devices.
    pub async fn host_connect(self: &Arc<Self>, id: String) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_connect(id))
    }

    /// Disconnect and cancel any automatic reconnect.
    pub async fn host_disconnect(self: &Arc<Self>, id: String) -> CmdResult<HostView> {
        on_rt!(self, hosts::host_disconnect(id))
    }

    pub fn host_add_vpn(&self, id: String, address: String, port: u16) -> CmdResult<HostView> {
        hosts::host_add_vpn(self, id, address, port)
    }

    pub fn host_remove_vpn(&self, id: String, endpoint_id: String) -> CmdResult<HostView> {
        hosts::host_remove_vpn(self, id, endpoint_id)
    }
}

// ---------------------------------------------------------------- keychain
impl App {
    pub fn keychain_list(&self) -> CmdResult<Vec<KeyView>> {
        keychain::keychain_list(&self.state)
    }

    /// Add a key. The private key is unlocked once to prove the passphrase
    /// fits; its text and passphrase go to the OS store only.
    pub async fn keychain_create(self: &Arc<Self>, input: KeyCreate) -> CmdResult<KeyView> {
        let app = self.clone();
        self.blocking(move || keychain::keychain_create(&app.state, input))
            .await
    }

    pub async fn keychain_update(
        self: &Arc<Self>,
        id: String,
        input: KeyUpdate,
    ) -> CmdResult<KeyView> {
        let app = self.clone();
        self.blocking(move || keychain::keychain_update(&app.state, &id, input))
            .await
    }

    /// Refused while a host still signs in with the key.
    pub async fn keychain_delete(self: &Arc<Self>, id: String) -> CmdResult<()> {
        let app = self.clone();
        self.blocking(move || keychain::keychain_delete(&app.state, &id))
            .await
    }

    /// Whether the key file at `path` asks for a passphrase. Fails when the file cannot be
    /// read or is no key file; the key text never leaves the backend.
    pub async fn key_file_is_encrypted(&self, path: std::path::PathBuf) -> CmdResult<bool> {
        self.blocking(move || keychain::key_file_is_encrypted(&path))
            .await
    }

    /// Read a key file for the "new key" form; nothing is stored.
    pub async fn keychain_import_file(&self, path: std::path::PathBuf) -> CmdResult<KeyImport> {
        self.blocking(move || keychain::keychain_import_file(&path))
            .await
    }
}

// ---------------------------------------------------------------- terminal
impl App {
    /// Open a shell. Output and the final [`TermEvent::Exit`] go to `sink`.
    pub async fn term_open(
        self: &Arc<Self>,
        shell: ShellRequest,
        cols: u16,
        rows: u16,
        sink: Sink<TermEvent>,
    ) -> CmdResult<u32> {
        on_rt!(self, terminal::term_open(shell, cols, rows, sink))
    }

    /// Queue input; applied in order by the session's writer.
    pub fn term_write(&self, id: u32, data: Vec<u8>) -> CmdResult<()> {
        terminal::term_write(self, id, data)
    }

    /// Queue a resize behind the input already queued.
    pub fn term_resize(&self, id: u32, cols: u16, rows: u16) -> CmdResult<()> {
        terminal::term_resize(self, id, cols, rows)
    }

    /// Kill the local shell or close the SSH channel. No `Exit` is promised.
    pub fn term_close(&self, id: u32) -> CmdResult<()> {
        terminal::term_close(self, id)
    }

    /// Installed WSL distributions.
    pub async fn wsl_distros(&self) -> CmdResult<Vec<String>> {
        self.run(terminal::wsl_distros()).await
    }

    /// The shells a local terminal can start: Windows PowerShell, PowerShell
    /// 7, Command Prompt, Git Bash and the WSL distributions, leaving out
    /// what is not installed. Looks every time it is called; the caller
    /// keeps the list.
    pub async fn local_shells(&self) -> CmdResult<Vec<LocalShell>> {
        self.run(terminal::local_shells()).await
    }
}

// ------------------------------------------------------------------- files
impl App {
    pub async fn sftp_list(self: &Arc<Self>, host_id: String, path: String) -> CmdResult<Listing> {
        on_rt!(self, files::sftp_list(host_id, path))
    }

    /// Returns the transfer id; progress and the result go to `sink`.
    pub async fn sftp_download(
        self: &Arc<Self>,
        host_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            files::sftp_download(host_id, remote_path, local_dir, overwrite, sink)
        )
    }

    pub async fn sftp_upload(
        self: &Arc<Self>,
        host_id: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            files::sftp_upload(host_id, local_path, remote_dir, overwrite, sink)
        )
    }

    /// Cancel any transfer (SFTP, device, cross-host) by id.
    pub fn transfer_cancel(&self, id: u32) {
        files::transfer_cancel(self, id)
    }
}

// ----------------------------------------------------------------- android
impl App {
    pub fn devices_list(&self) -> CmdResult<Vec<DeviceView>> {
        android::devices_list(self)
    }

    pub fn adb_status(&self) -> AdbStatus {
        android::adb_status(self)
    }

    pub async fn device_info(self: &Arc<Self>, route_id: String) -> CmdResult<DeviceInfo> {
        on_rt!(self, android::device_info(route_id))
    }

    /// Run a user-typed shell command; `expected_serial` is what the user confirmed.
    pub async fn device_shell(
        self: &Arc<Self>,
        route_id: String,
        expected_serial: String,
        command: String,
    ) -> CmdResult<ShellResult> {
        on_rt!(
            self,
            android::device_shell(route_id, expected_serial, command)
        )
    }

    pub fn device_rename(&self, device_id: String, name: String) -> CmdResult<()> {
        android::device_rename(self, device_id, name)
    }

    /// Forget a device that is not currently connected.
    pub fn device_forget(&self, route_id: String) -> CmdResult<()> {
        android::device_forget(self, route_id)
    }

    /// Phones advertising Wireless debugging on the chosen host's network.
    pub async fn wireless_scan(self: &Arc<Self>, host_id: String) -> CmdResult<Vec<MdnsService>> {
        on_rt!(self, android::wireless_scan(host_id))
    }

    /// Pair with the code shown on the phone (never logged or stored).
    pub async fn wireless_pair(
        self: &Arc<Self>,
        host_id: String,
        address: String,
        code: String,
    ) -> CmdResult<String> {
        on_rt!(self, android::wireless_pair(host_id, address, code))
    }

    pub async fn wireless_connect(
        self: &Arc<Self>,
        host_id: String,
        address: String,
    ) -> CmdResult<String> {
        on_rt!(self, android::wireless_connect(host_id, address))
    }

    pub async fn wireless_disconnect(self: &Arc<Self>, route_id: String) -> CmdResult<String> {
        on_rt!(self, android::wireless_disconnect(route_id))
    }
}

// ------------------------------------------------------------ android tools
impl App {
    /// PNG bytes of the screen; also kept for [`App::device_screenshot_save`].
    pub async fn device_screenshot(self: &Arc<Self>, route_id: String) -> CmdResult<Vec<u8>> {
        on_rt!(self, android_tools::device_screenshot(route_id))
    }

    /// Write the last screenshot of `route_id` to `path`.
    pub async fn device_screenshot_save(
        self: &Arc<Self>,
        route_id: String,
        path: String,
    ) -> CmdResult<()> {
        on_rt!(self, android_tools::device_screenshot_save(route_id, path))
    }

    /// Last 500 lines, then live, until stopped or the device goes away.
    pub async fn logcat_start(
        self: &Arc<Self>,
        route_id: String,
        sink: Sink<LogcatEvent>,
    ) -> CmdResult<u32> {
        on_rt!(self, android_tools::logcat_start(route_id, sink))
    }

    /// Stop on the viewer's request (no `Ended` event follows).
    pub fn logcat_stop(&self, id: u32) {
        android_tools::logcat_stop(self, id)
    }

    /// User-installed packages, sorted.
    pub async fn device_packages(self: &Arc<Self>, route_id: String) -> CmdResult<Vec<String>> {
        on_rt!(self, android_tools::device_packages(route_id))
    }

    pub async fn device_uninstall(
        self: &Arc<Self>,
        route_id: String,
        expected_serial: String,
        package: String,
    ) -> CmdResult<String> {
        on_rt!(
            self,
            android_tools::device_uninstall(route_id, expected_serial, package)
        )
    }

    pub async fn device_install_apk(
        self: &Arc<Self>,
        route_id: String,
        expected_serial: String,
        local_path: String,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            android_tools::device_install_apk(route_id, expected_serial, local_path, sink)
        )
    }

    pub async fn device_files_list(
        self: &Arc<Self>,
        route_id: String,
        path: String,
    ) -> CmdResult<Vec<SyncEntry>> {
        on_rt!(self, android_tools::device_files_list(route_id, path))
    }

    pub async fn device_push(
        self: &Arc<Self>,
        route_id: String,
        expected_serial: String,
        local_path: String,
        remote_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            android_tools::device_push(
                route_id,
                expected_serial,
                local_path,
                remote_dir,
                overwrite,
                sink
            )
        )
    }

    pub async fn device_pull(
        self: &Arc<Self>,
        route_id: String,
        remote_path: String,
        local_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            android_tools::device_pull(route_id, remote_path, local_dir, overwrite, sink)
        )
    }
}

// ------------------------------------------------------------------ mirror
impl App {
    /// Start mirroring. Video and the final [`MirrorEvent::Ended`] go to `sink`.
    pub async fn mirror_start(
        self: &Arc<Self>,
        route_id: String,
        sink: Sink<MirrorEvent>,
    ) -> CmdResult<MirrorStarted> {
        on_rt!(self, mirror::mirror_start(route_id, sink))
    }

    /// Queue one input event; written in order, queued touch moves merged.
    pub fn mirror_input(&self, id: u32, input: Input) -> CmdResult<()> {
        mirror::mirror_input(self, id, input)
    }

    /// Stop on the viewer's request (no `Ended` event follows).
    pub fn mirror_stop(&self, id: u32) {
        mirror::mirror_stop(self, id)
    }
}

// ----------------------------------------------------------------- monitor
impl App {
    /// One metrics sample of a host (this PC or a connected Linux host).
    pub async fn host_metrics(self: &Arc<Self>, host_id: String) -> CmdResult<Metrics> {
        on_rt!(self, monitor::host_metrics(host_id))
    }
}

// ----------------------------------------------------------------- desktop
impl App {
    pub async fn desktop_info(self: &Arc<Self>, host_id: String) -> CmdResult<DesktopInfo> {
        on_rt!(self, desktop::desktop_info(host_id))
    }

    /// Start x11vnc for the host's X11 session (loopback only, one client,
    /// one-time password) and say where to reach it.
    pub async fn desktop_open(self: &Arc<Self>, host_id: String) -> CmdResult<VncTarget> {
        on_rt!(self, desktop::desktop_open(host_id))
    }

    /// Stop the session's x11vnc. Safe to call twice.
    pub async fn desktop_close(self: &Arc<Self>, id: u32) -> CmdResult<()> {
        on_rt!(self, desktop::desktop_close(id))
    }
}

// --------------------------------------------------------------- workflows
impl App {
    /// Saved workflows of the Workflows canvas, by name.
    pub fn workflows_list(&self) -> CmdResult<Vec<WorkflowView>> {
        workflows::workflows_list(self)
    }

    /// Create (`id` = `None`) or replace a workflow. Drafts with problems
    /// are saved as drawn; [`workflow_problems`] lists what blocks a run.
    pub fn workflow_save(
        &self,
        id: Option<String>,
        name: String,
        graph: &Workflow,
    ) -> CmdResult<WorkflowView> {
        workflows::workflow_save(self, id, name, graph)
    }

    pub fn workflow_delete(&self, id: &str) -> CmdResult<()> {
        workflows::workflow_delete(self, id)
    }

    /// Run a saved workflow; returns the run id. Steps, log lines and the
    /// final [`FlowEvent::Done`] go to `sink`; [`App::flow_cancel`] stops it.
    pub async fn workflow_run(
        self: &Arc<Self>,
        request: WorkflowRunRequest,
        sink: Sink<FlowEvent>,
    ) -> CmdResult<String> {
        on_rt!(self, wfrun::workflow_run(request, sink))
    }
}

// -------------------------------------------------------------- workspaces
impl App {
    pub fn workspaces_list(&self) -> CmdResult<Vec<WorkspaceRecord>> {
        workspaces::workspaces_list(self)
    }

    pub fn workspace_save(&self, input: WorkspaceSave) -> CmdResult<WorkspaceRecord> {
        workspaces::workspace_save(self, input)
    }

    pub fn workspace_delete(&self, id: String) -> CmdResult<()> {
        workspaces::workspace_delete(self, id)
    }
}

// --------------------------------------------------------------- crosshost
impl App {
    /// Linux host file → Android device folder, without a local copy.
    #[allow(clippy::too_many_arguments)]
    pub async fn xfer_host_to_device(
        self: &Arc<Self>,
        host_id: String,
        remote_path: String,
        route_id: String,
        expected_serial: String,
        device_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            crosshost::xfer_host_to_device(
                host_id,
                remote_path,
                route_id,
                expected_serial,
                device_dir,
                overwrite,
                sink
            )
        )
    }

    /// Android device file → Linux host folder, without a local copy.
    pub async fn xfer_device_to_host(
        self: &Arc<Self>,
        route_id: String,
        device_path: String,
        host_id: String,
        remote_dir: String,
        overwrite: bool,
        sink: Sink<TransferEvent>,
    ) -> CmdResult<u32> {
        on_rt!(
            self,
            crosshost::xfer_device_to_host(
                route_id,
                device_path,
                host_id,
                remote_dir,
                overwrite,
                sink
            )
        )
    }
}

// -------------------------------------------------------- flow and recipes
impl App {
    pub async fn flutter_info(self: &Arc<Self>, host_id: String) -> CmdResult<FlutterInfo> {
        on_rt!(self, flow::flutter_info(host_id))
    }

    /// Start a build → deploy run; returns the run id. Steps, log lines and
    /// the final [`FlowEvent::Done`] go to `sink`.
    pub async fn flow_start(
        self: &Arc<Self>,
        spec: FlowSpec,
        sink: Sink<FlowEvent>,
    ) -> CmdResult<String> {
        on_rt!(self, flow::flow_start(spec, sink))
    }

    /// Cancel a flow or recipe run.
    pub fn flow_cancel(&self, run_id: &str) {
        flow::flow_cancel(self, run_id)
    }

    /// The last 50 runs.
    pub fn flow_history(&self) -> CmdResult<Vec<RunRecord>> {
        flow::flow_history(self)
    }

    pub fn recipes_list(&self) -> CmdResult<Vec<RecipeRecord>> {
        recipes::recipes_list(self)
    }

    pub fn recipe_save(&self, input: RecipeSave) -> CmdResult<RecipeRecord> {
        recipes::recipe_save(self, input)
    }

    pub fn recipe_delete(&self, id: String) -> CmdResult<()> {
        recipes::recipe_delete(self, id)
    }

    /// Run a saved recipe against its target device; returns the run id.
    pub async fn recipe_run(
        self: &Arc<Self>,
        recipe_id: String,
        expected_serial: String,
        sink: Sink<FlowEvent>,
    ) -> CmdResult<String> {
        on_rt!(self, recipes::recipe_run(recipe_id, expected_serial, sink))
    }
}
