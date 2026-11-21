import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  devicesList,
  DeviceView,
  errorText,
  FlowEvent,
  flowCancel,
  flowHistory,
  flowStart,
  flutterInfo,
  FlutterInfo,
  RunRecord,
} from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { routeText } from "../devices/DevicesView";

const STEP_LABELS: Record<string, string> = {
  validate: "Check SDK, project and target",
  build: "Build APK",
  artifact: "Locate and fingerprint APK",
  install: "Install on target device",
  launch: "Launch app",
  logcat: "App log (15 s)",
};
const ICON: Record<string, string> = { pending: "○", running: "◌", succeeded: "✓", failed: "✕", cancelled: "■", skipped: "–" };
const MAX_LOG = 2000;

interface StepView {
  name: string;
  status: string;
  detail: string | null;
}

/** Build on one host, install on a phone behind any host (FLOW-01..03). */
export function DeployView() {
  const { hosts } = useUi();
  const local = hosts.find((h) => h.is_local);
  const [flutter, setFlutter] = useState<FlutterInfo | null>(null);
  const [flutterErr, setFlutterErr] = useState<string | null>(null);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [project, setProject] = useState("");
  const [mode, setMode] = useState<"release" | "debug">("release");
  const [routeId, setRouteId] = useState("");
  const [launch, setLaunch] = useState(true);
  const [logcat, setLogcat] = useState(true);
  const [confirming, setConfirming] = useState(false);
  const [runId, setRunId] = useState<string | null>(null);
  const [steps, setSteps] = useState<StepView[]>([]);
  const [done, setDone] = useState<{ status: string; message: string } | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [history, setHistory] = useState<RunRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const logBox = useRef<HTMLPreElement>(null);

  const loadHistory = useCallback(() => flowHistory().then(setHistory, () => {}), []);

  useEffect(() => {
    if (!local) return;
    const t = setTimeout(() => {
      flutterInfo(local.id).then(setFlutter, (e) => setFlutterErr(errorText(e)));
      devicesList().then((d) => {
        const ok = d.filter((x) => x.state === "available");
        setDevices(ok);
        setRouteId((r) => r || ok[0]?.route_id || "");
      }, () => {});
      void loadHistory();
    }, 0);
    return () => clearTimeout(t);
  }, [local, loadHistory]);

  useEffect(() => {
    if (logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
  }, [log]);

  const target = useMemo(() => devices.find((d) => d.route_id === routeId), [devices, routeId]);
  const running = runId !== null && done === null;

  const browse = async () => {
    const dir = await open({ title: "Flutter project folder", directory: true, multiple: false });
    if (typeof dir === "string") setProject(dir);
  };

  const start = async () => {
    setConfirming(false);
    if (!local || !target) return;
    setSteps(Object.keys(STEP_LABELS).map((name) => ({ name, status: "pending", detail: null })));
    setLog([]);
    setDone(null);
    setError(null);
    try {
      const id = await flowStart(
        { build_host_id: local.id, project_dir: project, mode, route_id: target.route_id, expected_serial: target.serial, launch, logcat },
        (ev: FlowEvent) => {
          if (ev.kind === "step") setSteps((s) => s.map((x, i) => (i === ev.ordinal ? { name: ev.name, status: ev.status, detail: ev.detail } : x)));
          else if (ev.kind === "log") setLog((l) => (l.length >= MAX_LOG ? l.slice(-MAX_LOG + 1) : l).concat(ev.text));
          else {
            setDone({ status: ev.status, message: ev.message });
            void loadHistory();
          }
        },
      );
      setRunId(id);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <section className="view padded deploy">
      <header className="view-header">
        <div>
          <h1>Deploy</h1>
          <p className="muted">Build a Flutter app on this PC and install it on any phone — including phones plugged into another host.</p>
        </div>
      </header>

      <div className="deploy-form">
        <p className="small">
          Build host: <strong>{local?.display_name ?? "this PC"}</strong> ·{" "}
          {flutter ? <span className="ok-text">{flutter.version}</span> : <span className="bad-text">{flutterErr ?? "checking Flutter…"}</span>}
        </p>
        <label>
          Flutter project folder
          <div className="row">
            <input className="grow mono" value={project} onChange={(e) => setProject(e.target.value)} placeholder="D:\\projects\\my_app" spellCheck={false} />
            <button type="button" onClick={browse}>
              Browse…
            </button>
          </div>
        </label>
        <div className="row">
          <label className="inline">
            Mode
            <select value={mode} onChange={(e) => setMode(e.target.value as "release" | "debug")}>
              <option value="release">release (smaller)</option>
              <option value="debug">debug</option>
            </select>
          </label>
          <label className="inline grow">
            Target
            <select value={routeId} onChange={(e) => setRouteId(e.target.value)}>
              {devices.length === 0 && <option value="">No device available</option>}
              {devices.map((d) => (
                <option key={d.route_id} value={d.route_id}>
                  {d.display_name} — {routeText(d)}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="row">
          <label className="check">
            <input type="checkbox" checked={launch} onChange={(e) => setLaunch(e.target.checked)} /> Launch after install
          </label>
          <label className="check">
            <input type="checkbox" checked={logcat} disabled={!launch} onChange={(e) => setLogcat(e.target.checked)} /> Show app log
          </label>
          <span className="grow" />
          {running ? (
            <button className="danger" onClick={() => runId && flowCancel(runId)}>
              Cancel
            </button>
          ) : (
            <button className="primary" disabled={!flutter || !project.trim() || !target} onClick={() => setConfirming(true)}>
              Build &amp; deploy
            </button>
          )}
        </div>
        {error && <p className="banner bad small">{error}</p>}
      </div>

      {steps.length > 0 && (
        <div className="deploy-run">
          <ol className="steps" aria-label="Steps">
            {steps.map((s) => (
              <li key={s.name} className={`step-${s.status}`}>
                <span className="step-icon" aria-hidden>
                  {ICON[s.status] ?? "?"}
                </span>
                <span className="step-name">{STEP_LABELS[s.name] ?? s.name}</span>
                <span className="step-status">{s.status}</span>
                {s.detail && <span className="muted small step-detail">{s.detail}</span>}
              </li>
            ))}
          </ol>
          {done && (
            <p className={`banner small ${done.status === "succeeded" ? "ok" : "bad"}`}>
              {done.status === "succeeded" ? "✓ " : "✕ "}
              {done.message}
            </p>
          )}
          <pre className="output log" ref={logBox} aria-label="Build and app log">
            {log.join("\n")}
          </pre>
        </div>
      )}

      {history.length > 0 && (
        <>
          <h2>History</h2>
          <table className="grid full">
            <thead>
              <tr>
                <th>When</th>
                <th>Target</th>
                <th>Mode</th>
                <th>Result</th>
                <th className="num">Time</th>
              </tr>
            </thead>
            <tbody>
              {history.map((r) => (
                <tr key={r.id}>
                  <td className="small">{new Date(r.created_at).toLocaleString()}</td>
                  <td className="small">{r.summary.target}</td>
                  <td className="small">{r.summary.mode}</td>
                  <td className="small">
                    <span className={`badge ${r.status === "succeeded" ? "ok" : r.status === "running" ? "" : "bad"}`}>{r.status}</span> {r.summary.result}
                  </td>
                  <td className="num small">{r.summary.seconds !== undefined ? `${r.summary.seconds} s` : ""}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}

      {confirming && target && (
        <ConfirmDialog title="Build and install?" confirmLabel="Build & deploy" onCancel={() => setConfirming(false)} onConfirm={start}>
          <p className="small">
            Build <code>{project}</code> ({mode}) on {local?.display_name}
          </p>
          <p className="target">
            → {target.display_name} — {routeText(target)}
          </p>
          <p className="muted small">The app is installed (replacing any older version of the same package){launch ? " and launched" : ""}.</p>
        </ConfirmDialog>
      )}
    </section>
  );
}
