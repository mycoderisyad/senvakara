//! Multi-host Android registry and routed ADB actions (AND-01..04,
//! tasks AND-IMPL-01..06).
//!
//! - The local Windows ADB server is watched from app start.
//! - A Linux host's ADB server is watched while that host is connected, via
//!   an SSH local port-forward to its loopback-only port 5037 (ADR-014).
//! - Watching uses `track-devices` (push, no polling).
//! - Every action re-validates host + serial + transport id against the live
//!   server immediately before running, and never falls back to another
//!   device or host (docs/SECURITY.md §2, AGENTS.md rule 6).

use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
use senvakara_adapters_adb::{ensure_local_server, find_local_adb, AdbDevice, AdbServer};
use senvakara_adapters_ssh::SshConnection;
use senvakara_core::{
    AndroidTarget, CoreError, DeviceId, ErrorCode, HostId, ObservedAdbDevice, RouteId,
};
use senvakara_storage::DeviceRouteRecord;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

/// Live (in-memory) binding of a route to a server transport. Never persisted.
#[derive(Debug, Clone)]
struct LiveRoute {
    host_id: String,
    server: AdbServer,
    serial: String,
    transport_id: Option<u64>,
}

struct Watcher {
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WatchStatus {
    pub watching: bool,
    pub detail: String,
}

#[derive(Default)]
pub struct Android {
    local_adb: Mutex<Option<PathBuf>>,
    live: Mutex<HashMap<String, LiveRoute>>,
    watchers: Mutex<HashMap<String, Watcher>>,
    status: Mutex<HashMap<String, WatchStatus>>,
}

fn connection_type(d: &AdbDevice) -> &'static str {
    if d.is_network() {
        "wifi_adb"
    } else {
        "usb_adb"
    }
}

fn route_state(adb_state: &str) -> &'static str {
    match adb_state {
        "device" => "available",
        "unauthorized" => "unauthorized",
        _ => "offline",
    }
}

fn set_status(app: &AppHandle, host_id: &str, watching: bool, detail: impl Into<String>) {
    let detail = detail.into();
    app.state::<Android>()
        .status
        .lock()
        .unwrap()
        .insert(host_id.to_string(), WatchStatus { watching, detail });
}

/// Apply one device-list snapshot from `host_id`'s server to the registry.
fn reconcile(app: &AppHandle, host_id: &str, server: AdbServer, devices: &[AdbDevice]) {
    let state = app.state::<AppState>();
    let android = app.state::<Android>();
    let mut present = Vec::new();
    {
        let mut db = state.db.lock().unwrap();
        let mut live = android.live.lock().unwrap();
        live.retain(|_, r| r.host_id != host_id);
        for d in devices {
            match db.observe_route(
                host_id,
                connection_type(d),
                &d.serial,
                route_state(&d.state),
                d.model.as_deref(),
            ) {
                Ok(rec) => {
                    live.insert(
                        rec.route_id.clone(),
                        LiveRoute {
                            host_id: host_id.to_string(),
                            server,
                            serial: d.serial.clone(),
                            transport_id: d.transport_id,
                        },
                    );
                    present.push(rec.route_id);
                }
                Err(e) => logging::warn(&format!("observe route on {host_id}: {e}")),
            }
        }
        if let Err(e) = db.mark_missing_routes(host_id, &present) {
            logging::warn(&format!("mark missing routes on {host_id}: {e}"));
        }
    }
    let _ = app.emit("devices-changed", ());
}

/// Host went away or watching stopped: nothing on it is reachable any more.
fn mark_host_gone(app: &AppHandle, host_id: &str) {
    app.state::<Android>()
        .live
        .lock()
        .unwrap()
        .retain(|_, r| r.host_id != host_id);
    let _ = app
        .state::<AppState>()
        .db
        .lock()
        .unwrap()
        .mark_missing_routes(host_id, &[]);
    let _ = app.emit("devices-changed", ());
}

/// Follow `track-devices` until the stream ends. Returns why it ended.
async fn track_until_end(app: &AppHandle, host_id: &str, server: AdbServer) -> String {
    let mut tracker = match server.track().await {
        Ok(t) => t,
        Err(e) => return format!("{e:#}"),
    };
    set_status(
        app,
        host_id,
        true,
        format!("watching ADB server at {}", server.addr),
    );
    loop {
        match tracker.next().await {
            Ok(devices) => reconcile(app, host_id, server, &devices),
            Err(e) => return format!("ADB device stream ended: {e:#}"),
        }
    }
}

/// Start watching the local Windows ADB server; restarts with backoff if the
/// server dies (e.g. `adb kill-server`).
pub fn start_local(app: AppHandle) {
    let android = app.state::<Android>();
    let adb = find_local_adb();
    *android.local_adb.lock().unwrap() = adb.clone();
    let local_id = match app
        .state::<AppState>()
        .db
        .lock()
        .unwrap()
        .ensure_local_host("This PC")
    {
        Ok(id) => id,
        Err(e) => return logging::warn(&format!("local host: {e}")),
    };
    let Some(adb) = adb else {
        set_status(
            &app,
            &local_id,
            false,
            "adb.exe not found: install Android SDK platform-tools",
        );
        mark_host_gone(&app, &local_id);
        return;
    };
    logging::info(&format!("local adb: {}", adb.display()));
    let task_app = app.clone();
    let host_id = local_id.clone();
    let task = tauri::async_runtime::spawn(async move {
        let mut backoff = Duration::from_secs(2);
        loop {
            match ensure_local_server(&adb).await {
                Ok(server) => {
                    backoff = Duration::from_secs(2);
                    let why = track_until_end(&task_app, &host_id, server).await;
                    logging::warn(&format!("local adb watch stopped: {why}"));
                    set_status(&task_app, &host_id, false, why);
                }
                Err(e) => set_status(
                    &task_app,
                    &host_id,
                    false,
                    format!("local ADB server unavailable: {e:#}"),
                ),
            }
            mark_host_gone(&task_app, &host_id);
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
        }
    });
    android
        .watchers
        .lock()
        .unwrap()
        .insert(local_id, Watcher { tasks: vec![task] });
}

/// Start watching a connected Linux host's ADB server through the SSH tunnel.
pub async fn start_remote(
    app: &AppHandle,
    host_id: &str,
    conn: Arc<SshConnection>,
    adb_path: &str,
) {
    stop_host(app, host_id);
    if let Err(e) = conn.exec(&[adb_path, "start-server"]).await {
        set_status(
            app,
            host_id,
            false,
            format!("could not start adb on host: {e:#}"),
        );
        return;
    }
    let (addr, tunnel) = match conn.forward_local("127.0.0.1", 5037).await {
        Ok(x) => x,
        Err(e) => {
            set_status(
                app,
                host_id,
                false,
                format!("could not open ADB tunnel: {e:#}"),
            );
            return;
        }
    };
    let server = AdbServer { addr };
    let task_app = app.clone();
    let id = host_id.to_string();
    let watch = tauri::async_runtime::spawn(async move {
        let why = track_until_end(&task_app, &id, server).await;
        logging::warn(&format!("adb watch on {id} stopped: {why}"));
        set_status(&task_app, &id, false, why);
        mark_host_gone(&task_app, &id);
    });
    let tunnel = tauri::async_runtime::JoinHandle::Tokio(tunnel);
    app.state::<Android>().watchers.lock().unwrap().insert(
        host_id.to_string(),
        Watcher {
            tasks: vec![watch, tunnel],
        },
    );
    logging::info(&format!("adb tunnel for host {host_id} on {addr}"));
}

/// Stop watching a host (disconnect / delete) and mark its routes unavailable.
pub fn stop_host(app: &AppHandle, host_id: &str) {
    if let Some(w) = app
        .state::<Android>()
        .watchers
        .lock()
        .unwrap()
        .remove(host_id)
    {
        for t in w.tasks {
            t.abort();
        }
        set_status(app, host_id, false, "host disconnected");
        mark_host_gone(app, host_id);
    }
}

// ---------------------------------------------------------------- commands

#[derive(Debug, Serialize)]
pub struct DeviceView {
    #[serde(flatten)]
    pub record: DeviceRouteRecord,
    pub transport_id: Option<u64>,
}

#[tauri::command]
pub fn devices_list(
    state: State<'_, AppState>,
    android: State<'_, Android>,
) -> CmdResult<Vec<DeviceView>> {
    let records = state
        .db
        .lock()
        .unwrap()
        .list_device_routes()
        .map_err(|e| db_err("list devices", e))?;
    let live = android.live.lock().unwrap();
    Ok(records
        .into_iter()
        .map(|record| DeviceView {
            transport_id: live.get(&record.route_id).and_then(|l| l.transport_id),
            record,
        })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct AdbStatus {
    pub local_adb_path: Option<String>,
    pub hosts: HashMap<String, WatchStatus>,
}

#[tauri::command]
pub fn adb_status(android: State<'_, Android>) -> AdbStatus {
    AdbStatus {
        local_adb_path: android
            .local_adb
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.display().to_string()),
        hosts: android.status.lock().unwrap().clone(),
    }
}

/// Resolve a route to its live server transport and verify, against the
/// server *right now*, that the same serial is still on the same transport.
pub(crate) async fn resolve(
    state: &AppState,
    android: &Android,
    route_id: &str,
) -> CmdResult<(DeviceRouteRecord, AdbServer, u64)> {
    let record = state
        .db
        .lock()
        .unwrap()
        .get_route(route_id)
        .map_err(|e| db_err("resolve route", e))?;
    let live = android.live.lock().unwrap().get(route_id).cloned();
    let Some(live) = live else {
        return Err(CoreError::new(
            ErrorCode::DeviceOffline,
            format!(
                "{} is not connected to {} right now",
                record.display_name, record.host_name
            ),
        ));
    };
    let observed: Vec<AdbDevice> = live
        .server
        .devices()
        .await
        .map_err(|e| to_core("adb devices", e))?;
    let host_id =
        HostId::new(&record.host_id).ok_or_else(|| CoreError::internal("empty host id"))?;
    let target = AndroidTarget {
        host_id: host_id.clone(),
        device_id: DeviceId::new(&record.device_id)
            .ok_or_else(|| CoreError::internal("empty device id"))?,
        route_id: RouteId::new(&record.route_id)
            .ok_or_else(|| CoreError::internal("empty route id"))?,
        expected_serial: live.serial.clone(),
    };
    let obs: Vec<ObservedAdbDevice> = observed
        .iter()
        .map(|d| ObservedAdbDevice {
            host_id: host_id.clone(),
            serial: d.serial.clone(),
            state: d.state.clone(),
        })
        .collect();
    target.validate(&obs)?;
    let current = observed
        .iter()
        .find(|d| d.serial == live.serial)
        .and_then(|d| d.transport_id);
    match (live.transport_id, current) {
        (Some(expected), Some(now)) if expected == now => Ok((record, live.server, now)),
        _ => Err(CoreError::new(
            ErrorCode::RouteStale,
            "the device reconnected since the list was loaded; refresh and try again",
        )),
    }
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub route_id: String,
    pub host_name: String,
    pub serial: String,
    pub transport_id: u64,
    pub props: Vec<(String, String)>,
}

#[tauri::command]
pub async fn device_info(
    state: State<'_, AppState>,
    android: State<'_, Android>,
    route_id: String,
) -> CmdResult<DeviceInfo> {
    let (record, server, transport_id) = resolve(&state, &android, &route_id).await?;
    let mut props = Vec::new();
    for name in [
        "ro.product.manufacturer",
        "ro.product.model",
        "ro.product.device",
        "ro.build.version.release",
        "ro.build.version.sdk",
        "ro.build.display.id",
        "ro.product.cpu.abi",
    ] {
        let v = server
            .getprop(transport_id, name)
            .await
            .map_err(|e| to_core("getprop", e))?;
        props.push((name.to_string(), v));
    }
    Ok(DeviceInfo {
        route_id,
        host_name: record.host_name,
        serial: record.serial,
        transport_id,
        props,
    })
}

#[derive(Debug, Serialize)]
pub struct ShellResult {
    pub route_id: String,
    pub host_name: String,
    pub serial: String,
    pub output: String,
    pub truncated: bool,
}

/// Run a user-typed shell command on one device. The UI must show the target
/// and get confirmation first; `expected_serial` must match what the user saw.
#[tauri::command]
pub async fn device_shell(
    state: State<'_, AppState>,
    android: State<'_, Android>,
    route_id: String,
    expected_serial: String,
    command: String,
) -> CmdResult<ShellResult> {
    if command.len() > 4096 {
        return Err(CoreError::new(
            ErrorCode::InvalidInput,
            "command is too long",
        ));
    }
    let (record, server, transport_id) = resolve(&state, &android, &route_id).await?;
    if record.serial != expected_serial {
        return Err(CoreError::new(
            ErrorCode::RouteStale,
            "target changed since confirmation",
        ));
    }
    logging::info(&format!(
        "adb shell on route {route_id} (host {}, transport {transport_id})",
        record.host_id
    ));
    let out = server
        .shell(transport_id, &command, Duration::from_secs(20), 256 * 1024)
        .await
        .map_err(|e| to_core("adb shell", e))?;
    Ok(ShellResult {
        route_id,
        host_name: record.host_name,
        serial: record.serial,
        output: out.output,
        truncated: out.truncated,
    })
}

#[tauri::command]
pub fn device_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
    name: String,
) -> CmdResult<()> {
    state
        .db
        .lock()
        .unwrap()
        .rename_device(&device_id, &name)
        .map_err(|e| db_err("rename device", e))?;
    let _ = app.emit("devices-changed", ());
    Ok(())
}

/// Forget a device that is not currently connected.
#[tauri::command]
pub fn device_forget(
    app: AppHandle,
    state: State<'_, AppState>,
    android: State<'_, Android>,
    route_id: String,
) -> CmdResult<()> {
    if android.live.lock().unwrap().contains_key(&route_id) {
        return Err(CoreError::new(
            ErrorCode::Conflict,
            "unplug the device before forgetting it",
        ));
    }
    let db = state.db.lock().unwrap();
    let rec = db
        .get_route(&route_id)
        .map_err(|e| db_err("forget device", e))?;
    db.forget_device(&rec.device_id)
        .map_err(|e| db_err("forget device", e))?;
    drop(db);
    let _ = app.emit("devices-changed", ());
    Ok(())
}
