//! SFTP file commands (FILE-01, task CORE-06): browse, upload and download
//! with progress and cancellation. Transfers run in the background; the UI
//! gets `TransferEvent`s through a [`Sink`]. A folder is copied with
//! everything in it (FILE-DIR-01); host to host is in `hostcopy`.

use crate::logging;
use crate::sink::Sink;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_ssh::sftp::{self, Listing};
use senvakara_core::CoreError;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

pub(crate) async fn sftp_list(app: &Arc<App>, host_id: String, path: String) -> CmdResult<Listing> {
    let state = &app.state;
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

async fn start(
    app: &Arc<App>,
    host_id: String,
    source: String,
    destination: String,
    overwrite: bool,
    dir: Direction,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    let (state, transfers) = (&app.state, &app.transfers);
    // Connect up front so connection errors surface on the request itself.
    let conn = state.connection(&host_id).await?;
    let session = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    let id = transfers.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    transfers.cancel.lock().unwrap().insert(id, cancel.clone());
    logging::info(&format!("transfer {id} started ({dir:?}) host {host_id}"));

    let task_app = app.clone();
    app.rt.spawn(async move {
        let mut last = Instant::now() - Duration::from_secs(1);
        let ev = on_event.clone();
        let progress = move |done: u64, total: u64| {
            // Throttle to ~10 events/s; completion is reported separately.
            if last.elapsed() >= Duration::from_millis(100) {
                last = Instant::now();
                ev(TransferEvent {
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
            Direction::Download => {
                let local_dir = PathBuf::from(&destination);
                // A link to a folder is not a folder here: links are not followed.
                let is_dir = session
                    .symlink_metadata(source.clone())
                    .await
                    .is_ok_and(|meta| meta.file_type().is_dir());
                if is_dir {
                    sftp::download_dir(&session, &source, &local_dir, overwrite, &cancel, progress)
                        .await
                        .map(|copied| copied.describe())
                } else {
                    sftp::download(&session, &source, &local_dir, overwrite, &cancel, progress)
                        .await
                        .map(|p| p.display().to_string())
                }
            }
            Direction::Upload => {
                let local = PathBuf::from(&source);
                let is_dir = tokio::fs::symlink_metadata(&local)
                    .await
                    .is_ok_and(|meta| meta.is_dir());
                if is_dir {
                    sftp::upload_dir(&session, &local, &destination, overwrite, &cancel, progress)
                        .await
                        .map(|copied| copied.describe())
                } else {
                    sftp::upload(&session, &local, &destination, overwrite, &cancel, progress).await
                }
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
        on_event(event);
        task_app.transfers.cancel.lock().unwrap().remove(&id);
    });
    Ok(id)
}

pub(crate) async fn sftp_download(
    app: &Arc<App>,
    host_id: String,
    remote_path: String,
    local_dir: String,
    overwrite: bool,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    start(
        app,
        host_id,
        remote_path,
        local_dir,
        overwrite,
        Direction::Download,
        on_event,
    )
    .await
}

pub(crate) async fn sftp_upload(
    app: &Arc<App>,
    host_id: String,
    local_path: String,
    remote_dir: String,
    overwrite: bool,
    on_event: Sink<TransferEvent>,
) -> CmdResult<u32> {
    start(
        app,
        host_id,
        local_path,
        remote_dir,
        overwrite,
        Direction::Upload,
        on_event,
    )
    .await
}

/// Create the folder `name` in `dir` on a host. Returns its path.
pub(crate) async fn sftp_mkdir(
    app: &Arc<App>,
    host_id: String,
    dir: String,
    name: String,
) -> CmdResult<String> {
    let conn = app.state.connection(&host_id).await?;
    let session = conn.sftp().await.map_err(|e| to_core("sftp", e))?;
    sftp::mkdir(&session, &dir, name.trim())
        .await
        .map_err(|e| to_core("create folder", e))
}

pub(crate) fn transfer_cancel(app: &App, id: u32) {
    let transfers = &app.transfers;
    if let Some(flag) = transfers.cancel.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}
