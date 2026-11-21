//! Android tools on one routed device (AND-07, FILE-02, tasks AND-IMPL-10):
//! screenshot, logcat, app list, APK install/uninstall, file push/pull.
//!
//! Every command resolves the route against the live server first
//! (`android::resolve`), so nothing runs on a stale or substituted device.
//! Mutating commands also require the serial the user confirmed.

use crate::android::resolve;
use crate::events::LogcatEvent;
use crate::files::{TransferEvent, TransferState, Transfers};
use crate::logging;
use crate::sink::Sink;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_adb::sync::{self, SyncEntry};
use senvakara_adapters_adb::AdbServer;
use senvakara_adapters_ssh::sftp::safe_local_name;
use senvakara_adapters_ssh::shell_quote;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::DeviceRouteRecord;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::task::AbortHandle;

#[derive(Default)]
pub struct AndroidTools {
    /// Last screenshot per route, kept so "Save" writes exactly what was shown.
    screenshots: Mutex<HashMap<String, Vec<u8>>>,
    next_log: AtomicU32,
    logcats: Mutex<HashMap<u32, AbortHandle>>,
}

pub(crate) fn confirm_target(record: &DeviceRouteRecord, expected_serial: &str) -> CmdResult<()> {
    if record.serial != expected_serial {
        return Err(CoreError::new(
            ErrorCode::RouteStale,
            "target changed since confirmation",
        ));
    }
    Ok(())
}

pub(crate) fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

pub(crate) fn invalid(msg: &str) -> CoreError {
    CoreError::new(ErrorCode::InvalidInput, msg)
}

/// Absolute device path without NULs or `..` components.
pub(crate) fn check_device_path(p: &str) -> CmdResult<()> {
    if !p.starts_with('/') || p.contains('\0') || p.split('/').any(|c| c == "..") || p.len() > 1024
    {
        return Err(invalid(
            "device path must be absolute and must not contain '..'",
        ));
    }
    Ok(())
}

pub(crate) fn device_join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

// ---------------------------------------------------------------- screenshot

/// Capture the screen as PNG bytes.
pub(crate) async fn device_screenshot(app: &Arc<App>, route_id: String) -> CmdResult<Vec<u8>> {
    let state = &app.state;
    let android = &app.android;
    let tools = &app.tools;
    let (_, server, t) = resolve(state, android, &route_id).await?;
    let png = server
        .exec_bytes(t, "screencap -p", Duration::from_secs(20), 48 * 1024 * 1024)
        .await
        .map_err(|e| to_core("screenshot", e))?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(CoreError::internal(
            "device did not return a PNG (screen locked or secure window?)",
        ));
    }
    tools
        .screenshots
        .lock()
        .unwrap()
        .insert(route_id, png.clone());
    Ok(png)
}

/// Save the last captured screenshot of `route_id` to `path` (from a save dialog).
pub(crate) async fn device_screenshot_save(
    app: &Arc<App>,
    route_id: String,
    path: String,
) -> CmdResult<()> {
    let tools = &app.tools;
    let png = tools
        .screenshots
        .lock()
        .unwrap()
        .get(&route_id)
        .cloned()
        .ok_or_else(|| CoreError::new(ErrorCode::Conflict, "take a screenshot first"))?;
    tokio::fs::write(&path, png)
        .await
        .map_err(|e| to_core("save screenshot", e.into()))
}

// ---------------------------------------------------------------- logcat

/// Stream `logcat` output (last 500 lines, then live) until stopped.
pub(crate) async fn logcat_start(
    app: &Arc<App>,
    route_id: String,
    sink: Sink<LogcatEvent>,
) -> CmdResult<u32> {
    let state = &app.state;
    let android = &app.android;
    let tools = &app.tools;
    let (_, server, t) = resolve(state, android, &route_id).await?;
    let mut stream = server
        .open_service(t, "shell:logcat -v threadtime -T 500")
        .await
        .map_err(|e| to_core("logcat", e))?;
    let id = tools.next_log.fetch_add(1, Ordering::Relaxed) + 1;
    let task_app = app.clone();
    let task = app.rt.spawn(async move {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if !sink(LogcatEvent::Data(buf[..n].to_vec())) {
                        break;
                    }
                }
            }
        }
        task_app.tools.logcats.lock().unwrap().remove(&id);
        sink(LogcatEvent::Ended);
    });
    tools
        .logcats
        .lock()
        .unwrap()
        .insert(id, task.abort_handle());
    logging::info(&format!("logcat {id} started on route {route_id}"));
    Ok(id)
}

pub(crate) fn logcat_stop(app: &App, id: u32) {
    let tools = &app.tools;
    if let Some(task) = tools.logcats.lock().unwrap().remove(&id) {
        task.abort();
    }
}

// ---------------------------------------------------------------- apps

/// Third-party (user-installed) packages.
pub(crate) async fn device_packages(app: &Arc<App>, route_id: String) -> CmdResult<Vec<String>> {
    let state = &app.state;
    let android = &app.android;
    let (_, server, t) = resolve(state, android, &route_id).await?;
    let out = server
        .shell(
            t,
            "pm list packages -3",
            Duration::from_secs(20),
            1024 * 1024,
        )
        .await
        .map_err(|e| to_core("list packages", e))?;
    let mut pkgs: Vec<String> = out
        .output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("package:"))
        .map(str::to_string)
        .collect();
    pkgs.sort();
    Ok(pkgs)
}

pub fn valid_package(name: &str) -> bool {
    let mut parts = name.split('.');
    let first_ok = parts
        .next()
        .is_some_and(|p| p.chars().next().is_some_and(|c| c.is_ascii_alphabetic()));
    first_ok
        && name.contains('.')
        && name.len() <= 255
        && name
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

pub(crate) async fn device_uninstall(
    app: &Arc<App>,
    route_id: String,
    expected_serial: String,
    package: String,
) -> CmdResult<String> {
    let state = &app.state;
    let android = &app.android;
    if !valid_package(&package) {
        return Err(invalid("invalid package name"));
    }
    let (record, server, t) = resolve(state, android, &route_id).await?;
    confirm_target(&record, &expected_serial)?;
    logging::info(&format!("uninstall {package} on route {route_id}"));
    let out = server
        .shell(
            t,
            &format!("pm uninstall {package}"),
            Duration::from_secs(60),
            64 * 1024,
        )
        .await
        .map_err(|e| to_core("uninstall", e))?;
    let text = out.output.trim().to_string();
    if text.starts_with("Success") {
        Ok(text)
    } else {
        Err(CoreError::new(
            ErrorCode::Conflict,
            format!("uninstall failed: {text}"),
        ))
    }
}

// ---------------------------------------------------------------- transfers

pub(crate) fn progress_sender(id: u32, total: u64, ch: Sink<TransferEvent>) -> impl FnMut(u64) {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |done| {
        if last.elapsed() >= Duration::from_millis(100) {
            last = Instant::now();
            ch(TransferEvent {
                id,
                state: TransferState::Running,
                done,
                total,
                result_path: None,
                error: None,
            });
        }
    }
}

pub(crate) fn finish(
    app: &Arc<App>,
    id: u32,
    ch: &Sink<TransferEvent>,
    result: Result<String, CoreError>,
) {
    let event = match result {
        Ok(path) => TransferEvent {
            id,
            state: TransferState::Done,
            done: 0,
            total: 0,
            result_path: Some(path),
            error: None,
        },
        Err(ce) => {
            let st = if ce.code == ErrorCode::Cancelled {
                TransferState::Cancelled
            } else {
                TransferState::Failed
            };
            TransferEvent {
                id,
                state: st,
                done: 0,
                total: 0,
                result_path: None,
                error: Some(ce),
            }
        }
    };
    logging::info(&format!("device transfer {id} finished: {:?}", event.state));
    ch(event);
    app.transfers.cancel.lock().unwrap().remove(&id);
}

pub(crate) fn new_transfer(transfers: &Transfers) -> (u32, Arc<AtomicBool>) {
    let id = transfers.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.cancel.lock().unwrap().insert(id, cancel.clone());
    (id, cancel)
}

pub(crate) async fn push_file(
    server: AdbServer,
    t: u64,
    local: &Path,
    remote: &str,
    cancel: &AtomicBool,
    progress: impl FnMut(u64),
) -> anyhow::Result<()> {
    let mut src = tokio::fs::File::open(local).await?;
    let mut s = server.sync(t).await?;
    let result = sync::send(
        &mut s,
        &mut src,
        remote,
        0o644,
        now_secs(),
        cancel,
        progress,
    )
    .await;
    if result.is_err() {
        drop(s);
        // Do not leave a partial file behind on the device.
        let _ = server
            .shell(
                t,
                &format!("rm -f {}", shell_quote(remote)),
                Duration::from_secs(10),
                1024,
            )
            .await;
    } else {
        sync::quit(&mut s).await;
    }
    result
}

/// Install a local APK: push to a unique temp path, `pm install -r`, clean up.
pub(crate) async fn device_install_apk(
    app: &Arc<App>,
    route_id: String,
    expected_serial: String,
    local_path: String,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    let state = &app.state;
    let android = &app.android;
    let transfers = &app.transfers;
    let local = PathBuf::from(&local_path);
    if local
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("apk"))
        != Some(true)
    {
        return Err(invalid("choose an .apk file"));
    }
    let total = tokio::fs::metadata(&local)
        .await
        .map_err(|e| to_core("apk", e.into()))?
        .len();
    let (record, server, t) = resolve(state, android, &route_id).await?;
    confirm_target(&record, &expected_serial)?;
    let (id, cancel) = new_transfer(transfers);
    let remote = format!("/data/local/tmp/senvakara-{}.apk", now_secs());
    logging::info(&format!("install apk on route {route_id} (transfer {id})"));
    let task_app = app.clone();
    app.rt.spawn(async move {
        let progress = progress_sender(id, total, on_event.clone());
        let result = async {
            push_file(server, t, &local, &remote, &cancel, progress)
                .await
                .map_err(|e| to_core("push apk", e))?;
            let out = server
                .shell(
                    t,
                    &format!("pm install -r {remote}"),
                    Duration::from_secs(180),
                    64 * 1024,
                )
                .await
                .map_err(|e| to_core("pm install", e));
            let _ = server
                .shell(t, &format!("rm -f {remote}"), Duration::from_secs(10), 1024)
                .await;
            let text = out?.output.trim().to_string();
            if text.contains("Success") {
                Ok(format!(
                    "Installed on {} ({})",
                    record.display_name, record.host_name
                ))
            } else {
                Err(CoreError::new(
                    ErrorCode::Conflict,
                    format!("install failed: {text}"),
                ))
            }
        }
        .await;
        finish(&task_app, id, &on_event, result);
    });
    Ok(id)
}

pub(crate) async fn device_files_list(
    app: &Arc<App>,
    route_id: String,
    path: String,
) -> CmdResult<Vec<SyncEntry>> {
    let state = &app.state;
    let android = &app.android;
    check_device_path(&path)?;
    let (_, server, t) = resolve(state, android, &route_id).await?;
    let mut s = server.sync(t).await.map_err(|e| to_core("sync", e))?;
    let dir = if path.ends_with('/') {
        path
    } else {
        format!("{path}/")
    };
    let entries = sync::list(&mut s, &dir)
        .await
        .map_err(|e| to_core("list device files", e))?;
    sync::quit(&mut s).await;
    Ok(entries)
}

pub(crate) async fn device_push(
    app: &Arc<App>,
    route_id: String,
    expected_serial: String,
    local_path: String,
    remote_dir: String,
    overwrite: bool,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    let state = &app.state;
    let android = &app.android;
    let transfers = &app.transfers;
    check_device_path(&remote_dir)?;
    let local = PathBuf::from(&local_path);
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(safe_local_name)
        .ok_or_else(|| invalid("file name is not supported"))?
        .to_string();
    let total = tokio::fs::metadata(&local)
        .await
        .map_err(|e| to_core("push", e.into()))?
        .len();
    let (record, server, t) = resolve(state, android, &route_id).await?;
    confirm_target(&record, &expected_serial)?;
    let remote = device_join(&remote_dir, &name);
    if !overwrite {
        let mut s = server.sync(t).await.map_err(|e| to_core("sync", e))?;
        let st = sync::stat(&mut s, &remote)
            .await
            .map_err(|e| to_core("stat", e))?;
        sync::quit(&mut s).await;
        if st.exists() {
            return Err(CoreError::new(
                ErrorCode::Conflict,
                format!("{name} already exists on the device"),
            ));
        }
    }
    let (id, cancel) = new_transfer(transfers);
    let task_app = app.clone();
    app.rt.spawn(async move {
        let progress = progress_sender(id, total, on_event.clone());
        let result = push_file(server, t, &local, &remote, &cancel, progress)
            .await
            .map(|_| remote.clone())
            .map_err(|e| to_core("push", e));
        finish(&task_app, id, &on_event, result);
    });
    Ok(id)
}

pub(crate) async fn device_pull(
    app: &Arc<App>,
    route_id: String,
    remote_path: String,
    local_dir: String,
    overwrite: bool,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    let state = &app.state;
    let android = &app.android;
    let transfers = &app.transfers;
    check_device_path(&remote_path)?;
    let name = remote_path
        .rsplit('/')
        .next()
        .and_then(safe_local_name)
        .ok_or_else(|| invalid("device file name is not safe to save locally"))?
        .to_string();
    let target = PathBuf::from(&local_dir).join(&name);
    if target.exists() && !overwrite {
        return Err(CoreError::new(
            ErrorCode::Conflict,
            format!("{name} already exists in the destination folder"),
        ));
    }
    let (_, server, t) = resolve(state, android, &route_id).await?;
    let (id, cancel) = new_transfer(transfers);
    let task_app = app.clone();
    app.rt.spawn(async move {
        let part = target.with_file_name(format!("{name}.part"));
        let result = async {
            let mut s = server.sync(t).await?;
            let total = sync::stat(&mut s, &remote_path).await?.size as u64;
            let progress = progress_sender(id, total, on_event.clone());
            let mut dst = tokio::fs::File::create(&part).await?;
            sync::recv(&mut s, &remote_path, &mut dst, &cancel, progress).await?;
            sync::quit(&mut s).await;
            drop(dst);
            if overwrite && target.exists() {
                tokio::fs::remove_file(&target).await?;
            }
            tokio::fs::rename(&part, &target).await?;
            anyhow::Ok(target.display().to_string())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&part).await;
        }
        finish(
            &task_app,
            id,
            &on_event,
            result.map_err(|e| to_core("pull", e)),
        );
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        assert!(valid_package("dev.senvakara.testfixture"));
        assert!(valid_package("com.example.app_2"));
        for bad in [
            "",
            "nodots",
            "com..x",
            "1com.x",
            "com.x;reboot",
            "com.x y",
            "com.x/../y",
        ] {
            assert!(!valid_package(bad), "{bad:?}");
        }
    }

    #[test]
    fn device_paths() {
        assert!(check_device_path("/sdcard/Download").is_ok());
        assert!(check_device_path("sdcard").is_err());
        assert!(check_device_path("/sdcard/../data").is_err());
        assert_eq!(device_join("/sdcard/", "a"), "/sdcard/a");
        assert_eq!(device_join("/sdcard", "a"), "/sdcard/a");
    }
}
