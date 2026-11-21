//! Host metrics (MON-01, task MON-IMPL-01). Linux values are parsed from a
//! single read-only command over SSH (`/proc` + `df`), so no agent and no
//! root are needed. Rates (CPU %, network bytes/s) come from two snapshots.

use serde::Serialize;

/// The Linux snapshot command. Fixed text, no interpolation.
pub const LINUX_COMMAND: &str =
    "cat /proc/stat /proc/meminfo /proc/uptime /proc/loadavg /proc/net/dev; \
echo '@@DF'; df -P -B1 /; echo '@@NPROC'; nproc";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// Sum of all jiffies on the aggregate `cpu` line.
    pub cpu_total: u64,
    /// idle + iowait jiffies.
    pub cpu_idle: u64,
    pub cpu_count: u32,
    pub mem_total: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub uptime_secs: f64,
    pub load1: f64,
    /// Bytes over all non-loopback interfaces.
    pub net_rx: u64,
    pub net_tx: u64,
    pub disk_total: u64,
    pub disk_used: u64,
}

/// What the UI shows for one host at one moment.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Metrics {
    /// `None` on the first sample (no previous snapshot to diff against).
    pub cpu_percent: Option<f64>,
    pub cpu_count: u32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub net_rx_per_sec: Option<f64>,
    pub net_tx_per_sec: Option<f64>,
    pub uptime_secs: f64,
    pub load1: Option<f64>,
}

pub fn parse_linux(text: &str) -> Snapshot {
    let mut s = Snapshot::default();
    let mut section = "";
    for line in text.lines() {
        match line.trim() {
            "@@DF" => {
                section = "df";
                continue;
            }
            "@@NPROC" => {
                section = "nproc";
                continue;
            }
            _ => {}
        }
        match section {
            "df" => {
                // Filesystem 1-blocks Used Available Capacity Mounted
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() >= 6 && f[5] == "/" {
                    s.disk_total = f[1].parse().unwrap_or(0);
                    s.disk_used = f[2].parse().unwrap_or(0);
                }
            }
            "nproc" => s.cpu_count = line.trim().parse().unwrap_or(s.cpu_count),
            _ => parse_proc_line(&mut s, line),
        }
    }
    s
}

fn kb(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024
}

fn parse_proc_line(s: &mut Snapshot, line: &str) {
    if let Some(rest) = line.strip_prefix("cpu ") {
        let v: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        // user nice system idle iowait irq softirq steal (guest fields are already in user/nice)
        s.cpu_total = v.iter().take(8).sum();
        s.cpu_idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
    } else if line.starts_with("MemTotal:") {
        s.mem_total = kb(line);
    } else if line.starts_with("MemAvailable:") {
        s.mem_available = kb(line);
    } else if line.starts_with("SwapTotal:") {
        s.swap_total = kb(line);
    } else if line.starts_with("SwapFree:") {
        s.swap_free = kb(line);
    } else if let Some((iface, rest)) = line.split_once(':') {
        let iface = iface.trim();
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if f.len() >= 9 && !iface.is_empty() && !iface.contains(' ') && iface != "lo" {
            s.net_rx += f[0];
            s.net_tx += f[8];
        }
    } else {
        let f: Vec<&str> = line.split_whitespace().collect();
        // /proc/uptime: "16158.24 31731.18"; /proc/loadavg: "1.03 0.49 0.18 2/767 8500"
        if f.len() == 2 && s.uptime_secs == 0.0 {
            if let (Ok(up), Ok(_)) = (f[0].parse::<f64>(), f[1].parse::<f64>()) {
                s.uptime_secs = up;
            }
        } else if f.len() == 5 && f[3].contains('/') {
            s.load1 = f[0].parse().unwrap_or(0.0);
        }
    }
}

/// Derive display metrics; `prev` with its age in seconds enables rates.
pub fn derive(now: &Snapshot, prev: Option<(&Snapshot, f64)>, has_load: bool) -> Metrics {
    let mut m = Metrics {
        cpu_count: now.cpu_count,
        mem_used: now.mem_total.saturating_sub(now.mem_available),
        mem_total: now.mem_total,
        swap_used: now.swap_total.saturating_sub(now.swap_free),
        swap_total: now.swap_total,
        disk_used: now.disk_used,
        disk_total: now.disk_total,
        uptime_secs: now.uptime_secs,
        load1: has_load.then_some(now.load1),
        ..Default::default()
    };
    if let Some((p, secs)) = prev {
        let total = now.cpu_total.saturating_sub(p.cpu_total);
        let idle = now.cpu_idle.saturating_sub(p.cpu_idle);
        if total > 0 {
            m.cpu_percent =
                Some(((total - idle.min(total)) as f64 / total as f64 * 100.0).clamp(0.0, 100.0));
        }
        if secs > 0.0 && now.net_rx >= p.net_rx && now.net_tx >= p.net_tx {
            m.net_rx_per_sec = Some((now.net_rx - p.net_rx) as f64 / secs);
            m.net_tx_per_sec = Some((now.net_tx - p.net_tx) as f64 / secs);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "cpu  19400 450 17813 3173118 4596 8279 5932 0 0 0
cpu0 9000 200 9000 1500000 2000 4000 3000 0 0 0
MemTotal:        7557408 kB
MemAvailable:    6024200 kB
SwapTotal:       7557116 kB
SwapFree:        7557116 kB
16158.24 31731.18
1.03 0.49 0.18 2/767 8500
Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 47228835   21532    0    0    0     0          0         0 47228835   21532    0    0    0     0       0          0
  wlp2s0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0
@@DF
Filesystem     1-blocks        Used    Available Capacity Mounted on
/dev/sda3      237279117312 69628162048 164496904192      30% /
@@NPROC
2
";

    #[test]
    fn parses_fedora_sample() {
        let s = parse_linux(SAMPLE);
        assert_eq!(
            s.cpu_total,
            19400 + 450 + 17813 + 3173118 + 4596 + 8279 + 5932
        );
        assert_eq!(s.cpu_idle, 3173118 + 4596);
        assert_eq!(s.mem_total, 7557408 * 1024);
        assert_eq!(s.mem_available, 6024200 * 1024);
        assert_eq!(s.uptime_secs, 16158.24);
        assert_eq!(s.load1, 1.03);
        assert_eq!((s.net_rx, s.net_tx), (1000, 2000), "loopback excluded");
        assert_eq!(s.disk_total, 237279117312);
        assert_eq!(s.disk_used, 69628162048);
        assert_eq!(s.cpu_count, 2);
    }

    #[test]
    fn derives_rates_from_two_snapshots() {
        let a = Snapshot {
            cpu_total: 1000,
            cpu_idle: 800,
            net_rx: 0,
            net_tx: 0,
            mem_total: 100,
            mem_available: 40,
            ..Default::default()
        };
        let b = Snapshot {
            cpu_total: 1100,
            cpu_idle: 850,
            net_rx: 5000,
            net_tx: 1000,
            mem_total: 100,
            mem_available: 40,
            ..Default::default()
        };
        let m = derive(&b, Some((&a, 5.0)), true);
        assert_eq!(m.cpu_percent, Some(50.0));
        assert_eq!(m.net_rx_per_sec, Some(1000.0));
        assert_eq!(m.mem_used, 60);
        assert_eq!(derive(&b, None, false).cpu_percent, None);
    }

    #[test]
    fn counter_reset_does_not_produce_negative_rates() {
        let a = Snapshot {
            net_rx: 5000,
            ..Default::default()
        };
        let b = Snapshot {
            net_rx: 10,
            ..Default::default()
        };
        assert_eq!(derive(&b, Some((&a, 5.0)), true).net_rx_per_sec, None);
    }
}
