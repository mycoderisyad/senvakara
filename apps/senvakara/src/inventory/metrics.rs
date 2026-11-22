//! Host health figures worded for a host card: the meters and the note under them.

use senvakara_app::Metrics;

use crate::Meter;

fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

fn rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_048_576.0 {
        format!("{:.1} MB/s", bytes_per_sec / 1_048_576.0)
    } else {
        format!("{:.1} KB/s", bytes_per_sec / 1024.0)
    }
}

fn uptime(secs: f64) -> String {
    let minutes = (secs / 60.0) as u64;
    let (days, hours, minutes) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}

pub(super) fn meters(metrics: &Metrics) -> (Vec<Meter>, String) {
    let fraction = |used: u64, total: u64| {
        if total == 0 {
            0.0
        } else {
            used as f32 / total as f32
        }
    };
    let cpu = metrics.cpu_percent.unwrap_or(0.0);
    let list = vec![
        Meter {
            label: format!("CPU ({} cores)", metrics.cpu_count).into(),
            value: match metrics.cpu_percent {
                Some(percent) => format!("{percent:.0} %").into(),
                None => "measuring".into(),
            },
            fraction: (cpu / 100.0) as f32,
        },
        Meter {
            label: "Memory".into(),
            value: format!("{} of {}", gib(metrics.mem_used), gib(metrics.mem_total)).into(),
            fraction: fraction(metrics.mem_used, metrics.mem_total),
        },
        Meter {
            label: "System disk".into(),
            value: format!("{} of {}", gib(metrics.disk_used), gib(metrics.disk_total)).into(),
            fraction: fraction(metrics.disk_used, metrics.disk_total),
        },
    ];
    let mut note = Vec::new();
    if let (Some(rx), Some(tx)) = (metrics.net_rx_per_sec, metrics.net_tx_per_sec) {
        note.push(format!("Network {} down, {} up", rate(rx), rate(tx)));
    }
    note.push(format!("Up {}", uptime(metrics.uptime_secs)));
    if let Some(load) = metrics.load1 {
        note.push(format!("Load {load:.2}"));
    }
    if metrics.swap_used > 0 {
        note.push(format!("Swap {}", gib(metrics.swap_used)));
    }
    (list, note.join("  ·  "))
}
