//! Screen mirroring sessions (AND-06, tasks AND-IMPL-08/09). Video and
//! input travel over the device's own route (local ADB, or the Linux host's
//! ADB through the SSH tunnel). Sessions run only while a viewer is open and
//! are torn down on stop, on stream end, when the viewer is gone, or when
//! the route disappears.
//!
//! Video goes to the session's [`Sink`] and ends with [`MirrorEvent::Ended`].
//! Input is queued per session and written in order by one task; touch MOVE
//! events that pile up behind a slow link are merged, so the device always
//! gets the latest position instead of a growing backlog.

use crate::android::resolve;
use crate::events::MirrorEvent;
use crate::logging;
use crate::sink::Sink;
use crate::state::{to_core, CmdResult};
use crate::App;
use senvakara_adapters_scrcpy::control::{self, Input, ACTION_MOVE};
use senvakara_adapters_scrcpy::{start, verified_server, MirrorOptions};
use senvakara_core::{CoreError, ErrorCode};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::task::AbortHandle;

struct Session {
    route_id: String,
    sink: Sink<MirrorEvent>,
    input: UnboundedSender<Input>,
    tasks: Vec<AbortHandle>,
}

impl Session {
    fn abort(&self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

#[derive(Default)]
pub struct Mirrors {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, Session>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorStarted {
    pub id: u32,
    pub device_name: String,
    pub width: u32,
    pub height: u32,
}

/// Whether `next` makes `prev` pointless: two MOVE events of the same pointer
/// in a row (only the latest position matters).
fn supersedes(prev: &Input, next: &Input) -> bool {
    matches!(
        (prev, next),
        (
            Input::Touch { action: ACTION_MOVE, pointer_id: a, .. },
            Input::Touch { action: ACTION_MOVE, pointer_id: b, .. },
        ) if a == b
    )
}

/// Take what is already queued behind `first` and merge runs of MOVE events.
/// Returns the event to send now and, if the run ended, the event after it.
fn merge_moves(
    mut first: Input,
    mut next_queued: impl FnMut() -> Option<Input>,
) -> (Input, Option<Input>) {
    while let Some(next) = next_queued() {
        if supersedes(&first, &next) {
            first = next;
        } else {
            return (first, Some(next));
        }
    }
    (first, None)
}

pub(crate) async fn mirror_start(
    app: &Arc<App>,
    route_id: String,
    sink: Sink<MirrorEvent>,
) -> CmdResult<MirrorStarted> {
    let (record, server, transport_id) = resolve(&app.state, &app.android, &route_id).await?;
    let jar = verified_server().map_err(|e| to_core("scrcpy-server", e))?;
    let m = start(server, transport_id, jar, &MirrorOptions::default())
        .await
        .map_err(|e| to_core("start mirror", e))?;
    let mirrors = &app.mirrors;
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
    let mut video = m.video;
    let mut control = m.control;
    let mut control_rx = m.control_rx;
    let server_shell = m.server_shell;

    // Held until the session is registered, so a stream that ends at once
    // cannot run its clean-up before there is something to clean up.
    let mut sessions = mirrors.sessions.lock().unwrap();
    let video_app = app.clone();
    let video_sink = sink.clone();
    let video_task = app.rt.spawn(async move {
        // Owning the shell stream here ties the server's life to this task.
        let _server_shell = server_shell;
        let sink = video_sink;
        let reason = loop {
            match video.next().await {
                Ok(p) => {
                    if let Some(codec) = p.codec {
                        if !sink(MirrorEvent::Config(codec)) {
                            break "viewer closed".to_string();
                        }
                    }
                    if !sink(MirrorEvent::Packet {
                        key: p.key,
                        pts: p.pts,
                        data: p.data,
                    }) {
                        break "viewer closed".to_string();
                    }
                }
                Err(e) => break format!("video stream ended: {e:#}"),
            }
        };
        logging::info(&format!("mirror {id} ended: {reason}"));
        // Dropping the session ends its input writer; the drain task ends
        // when the server (stopped with this task) closes the control socket.
        let _session = video_app.mirrors.sessions.lock().unwrap().remove(&id);
        sink(MirrorEvent::Ended(reason));
    });

    let (input, mut queued) = unbounded_channel::<Input>();
    let input_task = app.rt.spawn(async move {
        let mut carried = None;
        loop {
            let first = match carried.take() {
                Some(i) => i,
                None => match queued.recv().await {
                    Some(i) => i,
                    None => break,
                },
            };
            let (now, after) = merge_moves(first, || queued.try_recv().ok());
            carried = after;
            if let Err(e) = control.send(&now).await {
                logging::warn(&format!("mirror {id}: input failed: {e:#}"));
                break;
            }
        }
    });
    // Device messages (clipboard) must be read so the server never blocks.
    let drain_task = app.rt.spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = control_rx.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
    });

    sessions.insert(
        id,
        Session {
            route_id,
            sink,
            input,
            tasks: vec![
                video_task.abort_handle(),
                input_task.abort_handle(),
                drain_task.abort_handle(),
            ],
        },
    );
    drop(sessions);
    Ok(started)
}

/// Queue one input event for the session; returns at once. Values the
/// device would refuse are rejected here.
pub(crate) fn mirror_input(app: &App, id: u32, input: Input) -> CmdResult<()> {
    control::encode(&input).map_err(|e| to_core("mirror input", e))?;
    let sent = app
        .mirrors
        .sessions
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|s| s.input.send(input).is_ok());
    if sent {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::Conflict,
            "mirror session is not running",
        ))
    }
}

/// Stop a session on the viewer's request (no `Ended` event follows).
pub(crate) fn mirror_stop(app: &App, id: u32) {
    let removed = app.mirrors.sessions.lock().unwrap().remove(&id);
    if let Some(s) = removed {
        logging::info(&format!("mirror {id} stopped on route {}", s.route_id));
        s.abort();
    }
}

/// Stop every mirror of a route (used when a device disappears).
pub(crate) fn stop_route(app: &App, route_id: &str) {
    let stopped: Vec<Session> = {
        let mut sessions = app.mirrors.sessions.lock().unwrap();
        let ids: Vec<u32> = sessions
            .iter()
            .filter(|(_, s)| s.route_id == route_id)
            .map(|(id, _)| *id)
            .collect();
        ids.iter().filter_map(|id| sessions.remove(id)).collect()
    };
    // Viewers are told only after the session lock is released.
    for s in stopped {
        s.abort();
        (s.sink)(MirrorEvent::Ended("device disconnected".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn touch(action: u8, pointer_id: u64, x: i32) -> Input {
        Input::Touch {
            action,
            pointer_id,
            x,
            y: 0,
            width: 100,
            height: 100,
            pressure: 1.0,
        }
    }

    /// What the writer task sends for a given backlog.
    fn drain(backlog: Vec<Input>) -> Vec<Input> {
        let mut queue: VecDeque<Input> = backlog.into();
        let mut sent = Vec::new();
        let mut carried = None;
        while let Some(first) = carried.take().or_else(|| queue.pop_front()) {
            let (now, after) = merge_moves(first, || queue.pop_front());
            carried = after;
            sent.push(now);
        }
        sent
    }

    #[test]
    fn consecutive_moves_collapse_to_the_latest() {
        let (down, up) = (control::ACTION_DOWN, control::ACTION_UP);
        let sent = drain(vec![
            touch(down, 1, 0),
            touch(ACTION_MOVE, 1, 1),
            touch(ACTION_MOVE, 1, 2),
            touch(ACTION_MOVE, 1, 3),
            touch(up, 1, 3),
        ]);
        assert_eq!(
            sent,
            [touch(down, 1, 0), touch(ACTION_MOVE, 1, 3), touch(up, 1, 3)]
        );
    }

    #[test]
    fn order_and_other_events_are_kept() {
        let backlog = vec![
            touch(ACTION_MOVE, 1, 1),
            Input::Text { text: "a".into() },
            touch(ACTION_MOVE, 1, 2),
            // Another pointer is another gesture: never merged.
            touch(ACTION_MOVE, 2, 9),
            Input::Rotate,
            Input::Rotate,
        ];
        assert_eq!(drain(backlog.clone()), backlog);
        assert!(drain(vec![]).is_empty());
    }
}
