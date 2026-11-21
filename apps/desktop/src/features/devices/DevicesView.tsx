import { FormEvent, useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AdbStatus,
  adbStatus,
  deviceForget,
  deviceInfo,
  DeviceInfo,
  deviceRename,
  deviceShell,
  devicesList,
  DeviceView,
  errorText,
  hostConnect,
  ShellResult,
} from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";

const ROUTE_LABEL = { usb_adb: "USB", wifi_adb: "Wi-Fi" } as const;

/** Route-aware state: text + icon, never colour alone (NFR-06). */
function StateBadge({ d }: { d: DeviceView }) {
  switch (d.state) {
    case "available":
      return <span className="badge ok">● Available</span>;
    case "unauthorized":
      return <span className="badge warn">! Unauthorized</span>;
    case "offline":
      return <span className="badge warn">◌ Offline</span>;
    default:
      return <span className="badge">○ Not connected</span>;
  }
}

/** "Fedora laptop · USB · 43bbf22b7d94" — shown before every action. */
export const routeText = (d: DeviceView) => `${d.host_name} · ${ROUTE_LABEL[d.connection_type]} · ${d.serial}`;

function ShellPanel({ device, onClose }: { device: DeviceView; onClose: () => void }) {
  const [command, setCommand] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<ShellResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setConfirming(false);
    setRunning(true);
    setError(null);
    try {
      setResult(await deviceShell(device.route_id, device.serial, command));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal wide" role="dialog" aria-modal="true" aria-labelledby="shell-title">
        <h2 id="shell-title">ADB shell · {device.display_name}</h2>
        <p className="target">Target: {routeText(device)}</p>
        <form
          className="row"
          onSubmit={(e: FormEvent) => {
            e.preventDefault();
            if (command.trim()) setConfirming(true);
          }}
        >
          <input
            className="grow mono"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="getprop ro.product.model"
            aria-label="Shell command"
            autoFocus
            spellCheck={false}
          />
          <button type="submit" className="primary" disabled={running || !command.trim()}>
            {running ? "Running…" : "Run"}
          </button>
        </form>
        {error && <p className="banner bad small">{error}</p>}
        {result && (
          <>
            <p className="muted small">
              Ran on {result.host_name} · {result.serial}
              {result.truncated ? " · output truncated" : ""}
            </p>
            <pre className="output">{result.output || "(no output)"}</pre>
          </>
        )}
        <div className="modal-actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
      {confirming && (
        <ConfirmDialog title="Run command on this device?" confirmLabel="Run" onCancel={() => setConfirming(false)} onConfirm={run}>
          <p>
            <code>{command}</code>
          </p>
          <p className="target">
            {device.display_name} — {routeText(device)}
          </p>
          <p className="muted small">The command runs with ADB shell permissions on this phone only.</p>
        </ConfirmDialog>
      )}
    </div>
  );
}

function InfoPanel({ device, onClose }: { device: DeviceView; onClose: () => void }) {
  const [info, setInfo] = useState<DeviceInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    deviceInfo(device.route_id).then(setInfo, (e) => setError(errorText(e)));
  }, [device.route_id]);
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="info-title">
        <h2 id="info-title">{device.display_name}</h2>
        <p className="target">{routeText(device)}</p>
        {!info && !error && <p>Reading device properties…</p>}
        {error && <p className="banner bad small">{error}</p>}
        {info && (
          <dl className="kv">
            <dt>Transport</dt>
            <dd>
              <code>{info.transport_id}</code>
            </dd>
            {info.props.map(([k, v]) => (
              <span key={k} style={{ display: "contents" }}>
                <dt>{k}</dt>
                <dd>
                  <code>{v || "—"}</code>
                </dd>
              </span>
            ))}
          </dl>
        )}
        <div className="modal-actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}

/** Android devices from every host in one list (INV-02, AND-03). */
export function DevicesView() {
  const { hosts, refreshHosts, putHost } = useUi();
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [status, setStatus] = useState<AdbStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [shellFor, setShellFor] = useState<DeviceView | null>(null);
  const [infoFor, setInfoFor] = useState<DeviceView | null>(null);
  const [forgetting, setForgetting] = useState<DeviceView | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [connecting, setConnecting] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [d, s] = await Promise.all([devicesList(), adbStatus()]);
      setDevices(d);
      setStatus(s);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    const t = setTimeout(() => void refresh(), 0);
    const un = listen("devices-changed", () => void refresh());
    return () => {
      clearTimeout(t);
      void un.then((u) => u());
    };
  }, [refresh]);

  const connectHost = async (id: string) => {
    setConnecting(id);
    try {
      putHost(await hostConnect(id));
      await refresh();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setConnecting(null);
      void refreshHosts();
    }
  };

  const saveName = async () => {
    if (!renaming) return;
    try {
      await deviceRename(renaming.id, renaming.name);
      setRenaming(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const local = hosts.find((h) => h.is_local);
  const remote = hosts.filter((h) => !h.is_local);

  return (
    <section className="view padded">
      <header className="view-header">
        <div>
          <h1>Devices</h1>
          <p className="muted">Android phones on every host, each shown with the host and connection it is reached through.</p>
        </div>
        <button onClick={() => void refresh()}>Refresh</button>
      </header>

      <div className="sources" aria-label="ADB sources">
        {local && (
          <div className="source">
            <strong>{local.display_name}</strong>{" "}
            {status?.local_adb_path ? (
              <span className={`badge ${status.hosts[local.id]?.watching ? "ok" : "warn"}`}>
                {status.hosts[local.id]?.watching ? "● watching" : `! ${status.hosts[local.id]?.detail ?? "starting"}`}
              </span>
            ) : (
              <span className="badge warn">! adb.exe not found — install Android SDK platform-tools</span>
            )}
          </div>
        )}
        {remote.map((h) => {
          const st = status?.hosts[h.id];
          const hasAdb = h.capabilities?.adb_path;
          return (
            <div key={h.id} className="source">
              <strong>{h.display_name}</strong>{" "}
              {st?.watching ? (
                <span className="badge ok">● watching</span>
              ) : !h.ssh?.fingerprint ? (
                <span className="badge warn">! host key not trusted</span>
              ) : h.capabilities && hasAdb === null ? (
                <span className="badge warn">! adb not installed on host</span>
              ) : (
                <>
                  <span className="badge">○ {st?.detail ?? "not connected"}</span>{" "}
                  <button onClick={() => connectHost(h.id)} disabled={connecting === h.id}>
                    {connecting === h.id ? "Connecting…" : "Connect"}
                  </button>
                </>
              )}
            </div>
          );
        })}
      </div>

      {error && <p className="banner bad small">{error}</p>}

      {devices.length === 0 ? (
        <div className="empty">
          <h2>No Android devices yet</h2>
          <p>Plug a phone with USB debugging enabled into this PC or into a connected Linux host.</p>
        </div>
      ) : (
        <div className="cards">
          {devices.map((d) => (
            <article key={d.route_id} className={`card ${d.state === "unavailable" ? "dim" : ""}`} aria-label={d.display_name}>
              <div className="card-head">
                <div>
                  {renaming?.id === d.device_id ? (
                    <form
                      className="row"
                      onSubmit={(e) => {
                        e.preventDefault();
                        void saveName();
                      }}
                    >
                      <input value={renaming.name} onChange={(e) => setRenaming({ id: d.device_id, name: e.target.value })} aria-label="Device name" autoFocus />
                      <button type="submit">Save</button>
                    </form>
                  ) : (
                    <h3>{d.display_name}</h3>
                  )}
                  <p className="route">
                    {d.host_name} · {ROUTE_LABEL[d.connection_type]}
                  </p>
                  <p className="muted small">
                    <code>{d.serial}</code>
                    {d.model && d.model !== d.display_name ? ` · ${d.model}` : ""}
                    {d.transport_id != null ? ` · transport ${d.transport_id}` : ""}
                  </p>
                </div>
                <StateBadge d={d} />
              </div>
              {d.state === "unauthorized" && (
                <p className="banner small warn-text">Unlock the phone and accept “Allow USB debugging” for {d.host_name}.</p>
              )}
              <div className="card-actions">
                <button onClick={() => setInfoFor(d)} disabled={d.state !== "available"}>
                  Info
                </button>
                <button onClick={() => setShellFor(d)} disabled={d.state !== "available"}>
                  Shell
                </button>
                <button onClick={() => setRenaming({ id: d.device_id, name: d.display_name })}>Rename</button>
                {d.state === "unavailable" && (
                  <button className="danger" onClick={() => setForgetting(d)}>
                    Forget
                  </button>
                )}
              </div>
            </article>
          ))}
        </div>
      )}

      {shellFor && <ShellPanel device={shellFor} onClose={() => setShellFor(null)} />}
      {infoFor && <InfoPanel device={infoFor} onClose={() => setInfoFor(null)} />}
      {forgetting && (
        <ConfirmDialog
          title={`Forget ${forgetting.display_name}?`}
          confirmLabel="Forget"
          danger
          onCancel={() => setForgetting(null)}
          onConfirm={async () => {
            const d = forgetting;
            setForgetting(null);
            try {
              await deviceForget(d.route_id);
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          <p>Removes its name and history from Senvakara. It appears again as a new device if plugged in later.</p>
        </ConfirmDialog>
      )}
    </section>
  );
}
