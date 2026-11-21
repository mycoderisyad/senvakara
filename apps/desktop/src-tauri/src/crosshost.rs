//! Cross-host transfers (FILE-03, task FILE-IMPL-01): stream a file between
//! a Linux host (SFTP) and an Android device on any route (ADB sync) without
//! a temporary copy on this PC. Size is checked after the copy; partial
//! files are removed on failure or cancel; existing files are never replaced
//! without `overwrite`.

use crate::android::{resolve, Android};
use crate::android_tools::{
    check_device_path, confirm_target, device_join, finish, invalid, new_transfer, now_secs,
    progress_sender,
};
use crate::files::{TransferEvent, Transfers};
use crate::logging;
use crate::state::{to_core, AppState, CmdResult};
use senvakara_adapters_adb::sync;
use senvakara_adapters_ssh::sftp::{remote_join, safe_local_name};
use senvakara_core::{CoreError, ErrorCode};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

fn file_name(path: &str) -> CmdResult<String> {
    path.rsplit('/')
        .next()
        .and_then(safe_local_name)
        .map(str::to_string)
        .ok_or_else(|| invalid("file name is not supported"))
}

/// Linux host file → Android device folder.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects handle and managed state
pub async fn xfer_host_to_device(
    app: AppHandle,
    state: State<'_, AppState>,
    android: State<'_, Android>,
    transfers: State<'_, Transfers>,
    host_id: String,
    remote_path: String,
    route_id: String,
    expected_serial: String,
    device_dir: String,
    overwrite: bool,
    on_event: Channel<TransferEvent>,
) -> CmdResult<u32> {
    check_device_path(&device_dir)?;
    let name = file_name(&remote_path)?;
    let conn = state.connection(&host_id).await?;
    let sftp = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    let total = sftp
        .metadata(remote_path.clone())
        .await
        .map_err(|e| to_core("stat source", e.into()))?
        .size
        .unwrap_or(0);
    let (record, server, t) = resolve(&state, &android, &route_id).await?;
    confirm_target(&record, &expected_serial)?;
    let target = device_join(&device_dir, &name);
    if !overwrite {
        let mut s = server.sync(t).await.map_err(|e| to_core("sync", e))?;
        let st = sync::stat(&mut s, &target)
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
    let (id, cancel) = new_transfer(&transfers);
    logging::info(&format!(
        "cross-host transfer {id}: host {host_id} → route {route_id}"
    ));
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let progress = progress_sender(id, total, on_event.clone());
        let result = async {
            let mut src = sftp
                .open(remote_path.clone())
                .await
                .map_err(|e| to_core("open source", e.into()))?;
            let mut s = server.sync(t).await.map_err(|e| to_core("sync", e))?;
            if let Err(e) = sync::send(
                &mut s,
                &mut src,
                &target,
                0o644,
                now_secs(),
                &cancel,
                progress,
            )
            .await
            {
                drop(s);
                let _ = server
                    .shell(
                        t,
                        &format!("rm -f {}", senvakara_adapters_ssh::shell_quote(&target)),
                        Duration::from_secs(10),
                        1024,
                    )
                    .await;
                return Err(to_core("send to device", e));
            }
            let st = sync::stat(&mut s, &target)
                .await
                .map_err(|e| to_core("verify", e))?;
            sync::quit(&mut s).await;
            if st.size as u64 != total {
                return Err(CoreError::new(
                    ErrorCode::InternalError,
                    format!("size mismatch after copy ({} vs {total} bytes)", st.size),
                ));
            }
            Ok(format!(
                "{target} on {} ({})",
                record.display_name, record.host_name
            ))
        }
        .await;
        finish(&task_app, id, &on_event, result);
    });
    Ok(id)
}

/// Android device file → Linux host folder.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects handle and managed state
pub async fn xfer_device_to_host(
    app: AppHandle,
    state: State<'_, AppState>,
    android: State<'_, Android>,
    transfers: State<'_, Transfers>,
    route_id: String,
    device_path: String,
    host_id: String,
    remote_dir: String,
    overwrite: bool,
    on_event: Channel<TransferEvent>,
) -> CmdResult<u32> {
    check_device_path(&device_path)?;
    let name = file_name(&device_path)?;
    let (_, server, t) = resolve(&state, &android, &route_id).await?;
    let conn = state.connection(&host_id).await?;
    let sftp = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    let target = remote_join(&remote_dir, &name);
    if !overwrite && sftp.try_exists(target.clone()).await.unwrap_or(false) {
        return Err(CoreError::new(
            ErrorCode::Conflict,
            format!("{name} already exists on the host"),
        ));
    }
    let (id, cancel) = new_transfer(&transfers);
    logging::info(&format!(
        "cross-host transfer {id}: route {route_id} → host {host_id}"
    ));
    let task_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let part = format!("{target}.part");
        let result = async {
            let mut s = server.sync(t).await.map_err(|e| to_core("sync", e))?;
            let total = sync::stat(&mut s, &device_path)
                .await
                .map_err(|e| to_core("stat source", e))?
                .size as u64;
            let progress = progress_sender(id, total, on_event.clone());
            let mut dst = sftp
                .create(part.clone())
                .await
                .map_err(|e| to_core("create on host", e.into()))?;
            let copied = sync::recv(&mut s, &device_path, &mut dst, &cancel, progress)
                .await
                .map_err(|e| to_core("read from device", e))?;
            sync::quit(&mut s).await;
            dst.shutdown()
                .await
                .map_err(|e| to_core("close on host", e.into()))?;
            if copied != total {
                return Err(CoreError::new(
                    ErrorCode::InternalError,
                    format!("size mismatch after copy ({copied} vs {total} bytes)"),
                ));
            }
            if overwrite {
                let _ = sftp.remove_file(target.clone()).await;
            }
            sftp.rename(part.clone(), target.clone())
                .await
                .map_err(|e| to_core("finalise on host", e.into()))?;
            Ok(target.clone())
        }
        .await;
        if result.is_err() {
            let _ = sftp.remove_file(part).await;
        }
        finish(&task_app, id, &on_event, result);
    });
    Ok(id)
}
