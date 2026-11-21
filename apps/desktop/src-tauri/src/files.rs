//! SFTP file commands (FILE-01, task CORE-06): browse, upload and download
//! with progress and cancellation. Transfers run in the background; the UI
//! gets `TransferEvent`s through a Channel.

use crate::logging;
use crate::state::{to_core, AppState, CmdResult};
use senvakara_adapters_ssh::sftp::{self, Listing};
use senvakara_core::CoreError;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
pub struct Transfers {
    pub(crate) next_id: AtomicU32,
    pub(crate) cancel: Mutex<HashMap<u32, Arc<AtomicBool>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferEvent {
    pub id: u32,
    pub state: TransferState,
    pub done: u64,
    pub total: u64,
    pub result_path: Option<String>,
    pub error: Option<CoreError>,
}

#[tauri::command]
pub async fn sftp_list(
    state: State<'_, AppState>,
    host_id: String,
    path: String,
) -> CmdResult<Listing> {
    let conn = state.connection(&host_id).await?;
    let session = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    let path = if path.trim().is_empty() {
        ".".to_string()
    } else {
        path
    };
    sftp::list(&session, &path)
        .await
        .map_err(|e| to_core("sftp list", e))
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Download,
    Upload,
}

#[allow(clippy::too_many_arguments)]
async fn start(
    app: AppHandle,
    transfers: &Transfers,
    state: &AppState,
    host_id: String,
    source: String,
    destination: String,
    overwrite: bool,
    dir: Direction,
    on_event: Channel<TransferEvent>,
) -> CmdResult<u32> {
    // Connect up front so connection errors surface on the request itself.
    let conn = state.connection(&host_id).await?;
    let session = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    let id = transfers.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.cancel.lock().unwrap().insert(id, cancel.clone());
    logging::info(&format!("transfer {id} started ({dir:?}) host {host_id}"));

    tauri::async_runtime::spawn(async move {
        let mut last = Instant::now() - Duration::from_secs(1);
        let ev = on_event.clone();
        let progress = move |done: u64, total: u64| {
            // Throttle to ~10 events/s; completion is reported separately.
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                let _ = ev.send(TransferEvent {
                    id,
                    state: TransferState::Running,
                    done,
                    total,
                    result_path: None,
                    error: None,
                });
            }
        };
        let result = match dir {
            Direction::Download => sftp::download(
                &session,
                &source,
                &PathBuf::from(&destination),
                overwrite,
                &cancel,
                progress,
            )
            .await
            .map(|p| p.display().to_string()),
            Direction::Upload => {
                sftp::upload(
                    &session,
                    &PathBuf::from(&source),
                    &destination,
                    overwrite,
                    &cancel,
                    progress,
                )
                .await
            }
        };
        let event = match result {
            Ok(path) => TransferEvent {
                id,
                state: TransferState::Done,
                done: 0,
                total: 0,
                result_path: Some(path),
                error: None,
            },
            Err(e) => {
                let ce = to_core("transfer", e);
                let st = if ce.code == senvakara_core::ErrorCode::Cancelled {
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
        logging::info(&format!("transfer {id} finished: {:?}", event.state));
        let _ = on_event.send(event);
        app.state::<Transfers>().cancel.lock().unwrap().remove(&id);
    });
    Ok(id)
}

#[tauri::command]
// Tauri injects AppHandle and managed state as parameters, hence the count.
#[allow(clippy::too_many_arguments)]
pub async fn sftp_download(
    app: AppHandle,
    transfers: State<'_, Transfers>,
    state: State<'_, AppState>,
    host_id: String,
    remote_path: String,
    local_dir: String,
    overwrite: bool,
    on_event: Channel<TransferEvent>,
) -> CmdResult<u32> {
    start(
        app,
        &transfers,
        &state,
        host_id,
        remote_path,
        local_dir,
        overwrite,
        Direction::Download,
        on_event,
    )
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sftp_upload(
    app: AppHandle,
    transfers: State<'_, Transfers>,
    state: State<'_, AppState>,
    host_id: String,
    local_path: String,
    remote_dir: String,
    overwrite: bool,
    on_event: Channel<TransferEvent>,
) -> CmdResult<u32> {
    start(
        app,
        &transfers,
        &state,
        host_id,
        local_path,
        remote_dir,
        overwrite,
        Direction::Upload,
        on_event,
    )
    .await
}

#[tauri::command]
pub fn transfer_cancel(transfers: State<'_, Transfers>, id: u32) {
    if let Some(flag) = transfers.cancel.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}
