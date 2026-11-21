//! Host monitoring (MON-01, tasks MON-IMPL-01/02).
//!
//! - Metrics are pulled only while the UI asks (no background polling when
//!   nobody looks); Linux hosts via one read-only command over the existing
//!   SSH connection, this PC via `sysinfo`.
//! - A light heartbeat checks live SSH connections every 15 s and tears down
//!   everything bound to a host that stopped answering (REC-01 groundwork).

use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
// sysinfo is pinned to 0.38 because 0.39+ requires rustc 1.95 (toolchain here: 1.94).
use senvakara_metrics::{derive, parse_linux, Metrics, Snapshot, LINUX_COMMAND};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sysinfo::{Disks, Networks, System};
use tauri::{AppHandle, Emitter, Manager, State};

struct LocalSampler {
    sys: System,
    nets: Networks,
    last: Option<(Instant, u64, u64)>,
    primed: bool,
}

#[derive(Default)]
pub struct Monitor {
    remote_prev: Mutex<HashMap<String, (Snapshot, Instant)>>,
    local: Mutex<Option<LocalSampler>>,
}

fn local_metrics(monitor: &Monitor) -> CmdResult<Metrics> {
    let mut guard = monitor.local.lock().unwrap();
    if guard.is_none() {
        *guard = Some(LocalSampler {
            sys: System::new(),
            nets: Networks::new_with_refreshed_list(),
            last: None,
            primed: false,
        });
    }
    let s = guard.as_mut().unwrap();
    s.sys.refresh_cpu_usage();
    s.sys.refresh_memory();
    s.nets.refresh(true);
    let (rx, tx) = s.nets.iter().fold((0u64, 0u64), |(r, t), (_, n)| {
        (r + n.total_received(), t + n.total_transmitted())
    });
    let now = Instant::now();
    let (rx_rate, tx_rate) = match s.last {
        Some((at, prx, ptx)) if rx >= prx && tx >= ptx => {
            let secs = now.duration_since(at).as_secs_f64().max(0.001);
            (
                Some((rx - prx) as f64 / secs),
                Some((tx - ptx) as f64 / secs),
            )
        }
        _ => (None, None),
    };
    s.last = Some((now, rx, tx));
    // CPU usage needs two refreshes; the first sample has no meaningful value.
    let cpu = s.primed.then(|| s.sys.global_cpu_usage() as f64);
    s.primed = true;
    let sys_drive = std::env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".into())
        .to_uppercase();
    let (disk_total, disk_free) = Disks::new_with_refreshed_list()
        .iter()
        .find(|x| {
            x.mount_point()
                .to_string_lossy()
                .to_uppercase()
                .starts_with(&sys_drive)
        })
        .map(|x| (x.total_space(), x.available_space()))
        .unwrap_or((0, 0));
    Ok(Metrics {
        cpu_percent: cpu,
        cpu_count: s.sys.cpus().len() as u32,
        mem_used: s
            .sys
            .total_memory()
            .saturating_sub(s.sys.available_memory()),
        mem_total: s.sys.total_memory(),
        swap_used: s.sys.used_swap(),
        swap_total: s.sys.total_swap(),
        disk_used: disk_total.saturating_sub(disk_free),
        disk_total,
        net_rx_per_sec: rx_rate,
        net_tx_per_sec: tx_rate,
        uptime_secs: System::uptime() as f64,
        load1: None,
    })
}

#[tauri::command]
pub async fn host_metrics(
    state: State<'_, AppState>,
    monitor: State<'_, Monitor>,
    host_id: String,
) -> CmdResult<Metrics> {
    let host = state
        .db
        .lock()
        .unwrap()
        .get_host(&host_id)
        .map_err(|e| db_err("metrics", e))?;
    if host.is_local {
        return local_metrics(&monitor);
    }
    let conn = state.connection(&host_id).await?;
    let out = conn
        .exec(&["sh", "-c", LINUX_COMMAND])
        .await
        .map_err(|e| to_core("metrics", e))?;
    let snap = parse_linux(&out.stdout_str());
    let now = Instant::now();
    let prev = monitor
        .remote_prev
        .lock()
        .unwrap()
        .insert(host_id, (snap.clone(), now));
    Ok(derive(
        &snap,
        prev.as_ref()
            .map(|(p, at)| (p, now.duration_since(*at).as_secs_f64())),
        true,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct HostLost {
    pub host_id: String,
    pub reason: String,
}

/// Every 15 s, probe each live SSH connection; drop the ones that no longer
/// answer so their sessions, tunnels and device routes show as offline.
pub fn spawn_heartbeat(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let state = app.state::<AppState>();
            let conns: Vec<(
                String,
                std::sync::Arc<senvakara_adapters_ssh::SshConnection>,
            )> = state
                .conns
                .lock()
                .await
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (host_id, conn) in conns {
                let alive = !conn.is_closed()
                    && matches!(tokio::time::timeout(Duration::from_secs(6), conn.exec(&["true"])).await, Ok(Ok(o)) if o.success());
                if !alive {
                    logging::warn(&format!("heartbeat: host {host_id} stopped answering"));
                    crate::android::stop_host(&app, &host_id);
                    state.disconnect(&host_id).await;
                    let _ = app.emit(
                        "host-lost",
                        HostLost {
                            host_id: host_id.clone(),
                            reason: "no response to heartbeat".into(),
                        },
                    );
                    spawn_reconnect(app.clone(), host_id);
                }
            }
        }
    });
}

#[derive(Debug, Clone, Serialize)]
pub struct HostReconnect {
    pub host_id: String,
    pub attempt: u32,
}

const MAX_ATTEMPTS: u32 = 20;

/// Delay before attempt `n` (1-based): 2, 4, 8, 16, 32, then 60 s.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.min(6)).min(60))
}

/// Reconnect a host the user had connected, with bounded exponential backoff
/// (REC-01). Nothing is replayed: terminals and transfers that were running
/// stay ended; only the connection, capabilities and device watching return.
/// Stops at once on authentication failure (e.g. changed host key) and when
/// the user disconnects or removes the host.
pub fn spawn_reconnect(app: AppHandle, host_id: String) {
    let state = app.state::<AppState>();
    if !state.want_connected.lock().unwrap().contains(&host_id) {
        return;
    }
    if state
        .reconnecting
        .lock()
        .unwrap()
        .insert(host_id.clone(), 0)
        .is_some()
    {
        return; // already running
    }
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut outcome = "gave up after the maximum number of attempts".to_string();
        for attempt in 1..=MAX_ATTEMPTS {
            tokio::time::sleep(backoff(attempt)).await;
            if !state.want_connected.lock().unwrap().contains(&host_id) {
                outcome = "cancelled".into();
                break;
            }
            state
                .reconnecting
                .lock()
                .unwrap()
                .insert(host_id.clone(), attempt);
            let _ = app.emit(
                "host-reconnecting",
                HostReconnect {
                    host_id: host_id.clone(),
                    attempt,
                },
            );
            match crate::hosts::connect_and_watch(&app, &state, &host_id).await {
                Ok(()) => {
                    logging::info(&format!("host {host_id} reconnected on attempt {attempt}"));
                    state.reconnecting.lock().unwrap().remove(&host_id);
                    let _ = app.emit(
                        "host-reconnected",
                        HostReconnect {
                            host_id: host_id.clone(),
                            attempt,
                        },
                    );
                    return;
                }
                Err(e) if e.code == senvakara_core::ErrorCode::Unauthenticated => {
                    outcome = format!("stopped: {}", e.message);
                    break;
                }
                Err(_) => {}
            }
        }
        logging::warn(&format!("reconnect of host {host_id}: {outcome}"));
        state.reconnecting.lock().unwrap().remove(&host_id);
        let _ = app.emit(
            "host-lost",
            HostLost {
                host_id,
                reason: format!("reconnect {outcome}"),
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::backoff;
    use std::time::Duration;

    #[test]
    fn backoff_is_bounded() {
        let secs: Vec<u64> = (1..=8).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(backoff(100), Duration::from_secs(60));
    }
}
