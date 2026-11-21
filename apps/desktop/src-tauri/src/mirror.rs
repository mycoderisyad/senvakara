//! Screen mirroring sessions (AND-06, tasks AND-IMPL-08/09). Video and
//! input travel over the device's own route (local ADB, or the Linux host's
//! ADB through the SSH tunnel). Sessions run only while a pane is open and
//! are torn down on stop, on stream end, or when the route disappears.

use crate::android::{resolve, Android};
use crate::logging;
use crate::state::{to_core, AppState, CmdResult};
use senvakara_adapters_scrcpy::control::{ControlWriter, Input};
use senvakara_adapters_scrcpy::{load_verified_server, start, MirrorOptions};
use senvakara_core::{CoreError, ErrorCode};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::AsyncReadExt;

struct Session {
    route_id: String,
    control: Arc<tokio::sync::Mutex<ControlWriter>>,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Default)]
pub struct Mirrors {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Session>>,
}

#[derive(Debug, Serialize)]
pub struct MirrorStarted {
    pub id: u32,
    pub device_name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorEnded {
    pub id: u32,
    pub reason: String,
}

/// Messages on the frame channel:
/// `0x00 <codec string>` when the stream (re)configures, then
/// `0x01 <key:u8> <pts:u64 BE> <annex-b data>` per frame.
fn frame_message(key: bool, pts: u64, data: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(10 + data.len());
    m.push(1);
    m.push(key as u8);
    m.extend_from_slice(&pts.to_be_bytes());
    m.extend_from_slice(data);
    m
}

#[tauri::command]
pub async fn mirror_start(
    app: AppHandle,
    state: State<'_, AppState>,
    android: State<'_, Android>,
    mirrors: State<'_, Mirrors>,
    route_id: String,
    on_frame: Channel<InvokeResponseBody>,
) -> CmdResult<MirrorStarted> {
    let (record, server, transport_id) = resolve(&state, &android, &route_id).await?;
    let jar_path = app
        .path()
        .resolve("resources/scrcpy-server-v3.3.3", BaseDirectory::Resource)
        .map_err(|e| CoreError::internal(format!("scrcpy-server resource: {e}")))?;
    let jar = load_verified_server(&jar_path)
        .await
        .map_err(|e| to_core("scrcpy-server", e))?;
    let m = start(server, transport_id, &jar, &MirrorOptions::default())
        .await
        .map_err(|e| to_core("start mirror", e))?;
    let id = mirrors.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    logging::info(&format!(
        "mirror {id} started on route {route_id} (host {}, {}x{})",
        record.host_id, m.width, m.height
    ));

    let started = MirrorStarted {
        id,
        device_name: m.device_name.clone(),
        width: m.width,
        height: m.height,
    };
    let control = Arc::new(tokio::sync::Mutex::new(m.control));
    let mut video = m.video;
    let mut control_rx = m.control_rx;
    let server_shell = m.server_shell;

    let video_app = app.clone();
    let video_task = tauri::async_runtime::spawn(async move {
        // Owning the shell stream here ties the server's life to this task.
        let _server_shell = server_shell;
        let reason = loop {
            match video.next().await {
                Ok(p) => {
                    if let Some(codec) = p.codec {
                        let mut cfg = vec![0u8];
                        cfg.extend_from_slice(codec.as_bytes());
                        if on_frame.send(InvokeResponseBody::Raw(cfg)).is_err() {
                            break "viewer closed".to_string();
                        }
                    }
                    if on_frame
                        .send(InvokeResponseBody::Raw(frame_message(
                            p.key, p.pts, &p.data,
                        )))
                        .is_err()
                    {
                        break "viewer closed".to_string();
                    }
                }
                Err(e) => break format!("video stream ended: {e:#}"),
            }
        };
        logging::info(&format!("mirror {id} ended: {reason}"));
        video_app
            .state::<Mirrors>()
            .sessions
            .lock()
            .unwrap()
            .remove(&id);
        let _ = video_app.emit("mirror-ended", MirrorEnded { id, reason });
    });
    // Device messages (clipboard) must be read so the server never blocks.
    let drain_task = tauri::async_runtime::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = control_rx.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
    });

    mirrors.sessions.lock().unwrap().insert(
        id,
        Session {
            route_id,
            control,
            tasks: vec![video_task, drain_task],
        },
    );
    Ok(started)
}

#[tauri::command]
pub async fn mirror_input(mirrors: State<'_, Mirrors>, id: u32, input: Input) -> CmdResult<()> {
    let control = mirrors
        .sessions
        .lock()
        .unwrap()
        .get(&id)
        .map(|s| s.control.clone())
        .ok_or_else(|| CoreError::new(ErrorCode::Conflict, "mirror session is not running"))?;
    let result = control.lock().await.send(&input).await;
    result.map_err(|e| to_core("mirror input", e))
}

#[tauri::command]
pub fn mirror_stop(mirrors: State<'_, Mirrors>, id: u32) {
    if let Some(s) = mirrors.sessions.lock().unwrap().remove(&id) {
        logging::info(&format!("mirror {id} stopped on route {}", s.route_id));
        for t in s.tasks {
            t.abort();
        }
    }
}

/// Stop every mirror of a route (used when a device disappears).
pub fn stop_route(app: &AppHandle, route_id: &str) {
    let mirrors = app.state::<Mirrors>();
    let mut sessions = mirrors.sessions.lock().unwrap();
    let ids: Vec<u32> = sessions
        .iter()
        .filter(|(_, s)| s.route_id == route_id)
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if let Some(s) = sessions.remove(&id) {
            for t in s.tasks {
                t.abort();
            }
            let _ = app.emit(
                "mirror-ended",
                MirrorEnded {
                    id,
                    reason: "device disconnected".into(),
                },
            );
        }
    }
}
