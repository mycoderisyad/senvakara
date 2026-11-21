import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { devicesList, DeviceView, errorText, hostConnect, hostMetrics, HostView, Metrics } from "../../lib/ipc";
import { useUi } from "../../lib/store";

const INTERVALS = [2, 5, 10, 30];

const fmtBytes = (n: number) =>
  n >= 1e12 ? `${(n / 1e12).toFixed(1)} TB` : n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(0)} MB` : `${(n / 1e3).toFixed(0)} KB`;
const fmtRate = (n: number | null) => (n === null ? "—" : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB/s` : `${(n / 1e3).toFixed(1)} KB/s`);
const fmtUptime = (s: number) => {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`;
};

/** A labelled bar; the number is always shown as text (not colour only). */
function Bar({ label, used, total, text }: { label: string; used: number; total: number; text: string }) {
  const pct = total > 0 ? Math.min(100, (used / total) * 100) : 0;
  return (
    <div className="bar">
      <div className="bar-label">
        <span>{label}</span>
        <span>{text}</span>
      </div>
      <div className="bar-track" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
        <div className={`bar-fill ${pct > 90 ? "hot" : pct > 75 ? "warm" : ""}`} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function HostMetrics({ host, interval }: { host: HostView; interval: number }) {
  const [m, setM] = useState<Metrics | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stop = false;
    const tick = () =>
      hostMetrics(host.id).then(
        (x) => !stop && (setM(x), setError(null)),
        (e) => !stop && setError(errorText(e)),
      );
    const first = setTimeout(tick, 0);
    // A second quick sample gives CPU % and rates without waiting a full interval.
    const second = setTimeout(tick, 1000);
    const t = setInterval(tick, interval * 1000);
    return () => {
      stop = true;
      clearTimeout(first);
      clearTimeout(second);
      clearInterval(t);
    };
  }, [host.id, interval]);

  if (error) return <p className="banner bad small">{error}</p>;
  if (!m) return <p className="muted small">Reading metrics…</p>;
  return (
    <div className="metrics">
      <Bar label={`CPU (${m.cpu_count} cores)`} used={m.cpu_percent ?? 0} total={100} text={m.cpu_percent === null ? "…" : `${m.cpu_percent.toFixed(0)} %`} />
      <Bar label="Memory" used={m.mem_used} total={m.mem_total} text={`${fmtBytes(m.mem_used)} / ${fmtBytes(m.mem_total)}`} />
      <Bar label="System disk" used={m.disk_used} total={m.disk_total} text={`${fmtBytes(m.disk_used)} / ${fmtBytes(m.disk_total)}`} />
      <p className="muted small">
        Network ↓ {fmtRate(m.net_rx_per_sec)} · ↑ {fmtRate(m.net_tx_per_sec)} · up {fmtUptime(m.uptime_secs)}
        {m.load1 !== null ? ` · load ${m.load1.toFixed(2)}` : ""}
        {m.swap_total > 0 ? ` · swap ${fmtBytes(m.swap_used)}` : ""}
      </p>
    </div>
  );
}

/** Hosts with live metrics and a device summary (MON-01, DESIGN.md Overview). */
export function OverviewView() {
  const { hosts, refreshHosts, putHost, setView } = useUi();
  const [interval, setIntervalSecs] = useState(5);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const load = () => devicesList().then(setDevices, () => {});
    const first = setTimeout(load, 0);
    const un1 = listen("devices-changed", load);
    const un2 = listen("host-lost", () => void refreshHosts());
    return () => {
      clearTimeout(first);
      void un1.then((u) => u());
      void un2.then((u) => u());
    };
  }, [refreshHosts]);

  const connect = async (h: HostView) => {
    setBusy(h.id);
    try {
      putHost(await hostConnect(h.id));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const count = (s: DeviceView["state"]) => devices.filter((d) => d.state === s).length;

  return (
    <section className="view padded">
      <header className="view-header">
        <div>
          <h1>Overview</h1>
          <p className="muted">Host health and devices. Metrics are read only while this page is open.</p>
        </div>
        <label className="inline">
          Refresh every{" "}
          <select value={interval} onChange={(e) => setIntervalSecs(Number(e.target.value))}>
            {INTERVALS.map((s) => (
              <option key={s} value={s}>
                {s} s
              </option>
            ))}
          </select>
        </label>
      </header>
      {error && <p className="banner bad small">{error}</p>}

      <div className="summary" role="status">
        <button className="link" onClick={() => setView("devices")}>
          Devices: <strong>{count("available")}</strong> available · <strong>{count("unauthorized")}</strong> unauthorized ·{" "}
          <strong>{count("offline") + count("unavailable")}</strong> not connected
        </button>
      </div>

      <div className="cards">
        {hosts.map((h) => {
          const live = h.is_local || h.connected;
          return (
            <article key={h.id} className="card" aria-label={h.display_name}>
              <div className="card-head">
                <div>
                  <h3>{h.display_name}</h3>
                  <p className="muted small">{h.is_local ? "Windows · this PC" : h.capabilities?.os_pretty_name ?? `${h.ssh?.address}`}</p>
                </div>
                <span className={`badge ${live ? "ok" : ""}`}>{h.is_local ? "● This PC" : h.connected ? "● Connected" : "○ Not connected"}</span>
              </div>
              {live ? (
                <HostMetrics host={h} interval={interval} />
              ) : h.ssh?.fingerprint ? (
                <button onClick={() => connect(h)} disabled={busy === h.id}>
                  {busy === h.id ? "Connecting…" : "Connect to see metrics"}
                </button>
              ) : (
                <p className="muted small">Trust the host key in Hosts first.</p>
              )}
            </article>
          );
        })}
      </div>
    </section>
  );
}
