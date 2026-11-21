import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  DeviceEntry,
  deviceFilesList,
  deviceInstallApk,
  devicePackages,
  devicePull,
  devicePush,
  deviceScreenshot,
  deviceScreenshotSave,
  devicesList,
  deviceUninstall,
  DeviceView,
  errorCode,
  errorText,
  logcatStart,
  logcatStop,
  TransferEvent,
} from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { routeText } from "./DevicesView";

type Section = "screen" | "logcat" | "apps" | "files";

interface Job {
  id: number;
  label: string;
  event: TransferEvent;
}

interface Confirm {
  title: string;
  body: string;
  label: string;
  danger?: boolean;
  run: () => void;
}

const fmtSize = (n: number) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`);

function useJobs() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const onEvent = (label: string) => (ev: TransferEvent) =>
    setJobs((j) => (j.some((x) => x.id === ev.id) ? j.map((x) => (x.id === ev.id ? { ...x, event: ev } : x)) : [{ id: ev.id, label, event: ev }, ...j]));
  const track = (id: number, label: string) =>
    setJobs((j) => (j.some((x) => x.id === id) ? j : [{ id, label, event: { id, state: "running", done: 0, total: 0, result_path: null, error: null } }, ...j]));
  return { jobs, onEvent, track };
}

function Jobs({ jobs }: { jobs: Job[] }) {
  if (jobs.length === 0) return null;
  return (
    <aside className="activity" aria-label="Device transfers">
      {jobs.map((j) => {
        const { state, done, total, error, result_path } = j.event;
        const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : state === "done" ? 100 : 0;
        return (
          <div key={j.id} className="job">
            <div className="job-head">
              <span>{j.label}</span>
              <span className={`badge ${state === "done" ? "ok" : state === "running" ? "" : "bad"}`}>{state === "running" ? `${pct}%` : state}</span>
            </div>
            <progress max={100} value={pct} aria-label={`${j.label} progress`} />
            {result_path && <p className="muted small">{result_path}</p>}
            {error && state !== "cancelled" && <p className="banner bad small">{errorText(error)}</p>}
          </div>
        );
      })}
    </aside>
  );
}

function ScreenSection({ device }: { device: DeviceView }) {
  const [url, setUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => () => void (url && URL.revokeObjectURL(url)), [url]);

  const capture = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const buf = await deviceScreenshot(device.route_id);
      setUrl(URL.createObjectURL(new Blob([buf], { type: "image/png" })));
    } catch (e) {
      setMsg(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const saveShot = async () => {
    const path = await save({ title: "Save screenshot", defaultPath: `${device.display_name.replace(/[^\w.-]+/g, "_")}-${Date.now()}.png`, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (!path) return;
    try {
      await deviceScreenshotSave(device.route_id, path);
      setMsg(`Saved to ${path}`);
    } catch (e) {
      setMsg(errorText(e));
    }
  };

  return (
    <div className="section">
      <div className="row">
        <button className="primary" onClick={capture} disabled={busy}>
          {busy ? "Capturing…" : "Take screenshot"}
        </button>
        <button onClick={saveShot} disabled={!url}>
          Save…
        </button>
        <span className="muted small">Live mirroring with input arrives in the next M3 step (scrcpy).</span>
      </div>
      {msg && <p className="muted small">{msg}</p>}
      {url && <img className="shot" src={url} alt={`Screenshot of ${device.display_name}`} />}
    </div>
  );
}

const MAX_LINES = 5000;

function LogcatSection({ device }: { device: DeviceView }) {
  const [lines, setLines] = useState<string[]>([]);
  const [id, setId] = useState<number | null>(null);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const box = useRef<HTMLPreElement>(null);
  const partial = useRef("");
  const decoder = useRef(new TextDecoder());
  const idRef = useRef<number | null>(null);

  useEffect(() => {
    const un = listen<number>("logcat-ended", (e) => {
      if (e.payload === idRef.current) {
        idRef.current = null;
        setId(null);
      }
    });
    return () => {
      void un.then((u) => u());
      if (idRef.current !== null) void logcatStop(idRef.current);
    };
  }, []);

  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [lines, follow]);

  const start = async () => {
    setError(null);
    try {
      const newId = await logcatStart(device.route_id, (bytes) => {
        const text = partial.current + decoder.current.decode(bytes, { stream: true });
        const parts = text.split(/\r?\n/);
        partial.current = parts.pop() ?? "";
        if (parts.length) setLines((l) => l.concat(parts).slice(-MAX_LINES));
      });
      idRef.current = newId;
      setId(newId);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const stop = () => {
    if (id !== null) void logcatStop(id);
    idRef.current = null;
    setId(null);
  };

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return f ? lines.filter((l) => l.toLowerCase().includes(f)) : lines;
  }, [lines, filter]);

  return (
    <div className="section">
      <div className="row">
        {id === null ? (
          <button className="primary" onClick={start}>
            Start logcat
          </button>
        ) : (
          <button onClick={stop}>Stop</button>
        )}
        <button onClick={() => setLines([])}>Clear</button>
        <input className="grow mono" value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Filter (text)" aria-label="Filter log lines" />
        <label className="check">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> Follow
        </label>
      </div>
      {error && <p className="banner bad small">{error}</p>}
      <p className="muted small">
        {id !== null ? "● streaming" : "○ stopped"} · {shown.length} of {lines.length} lines (keeps last {MAX_LINES})
      </p>
      <pre className="output log" ref={box} aria-label="Logcat output">
        {shown.join("\n")}
      </pre>
    </div>
  );
}

function AppsSection({ device, ask }: { device: DeviceView; ask: (c: Confirm) => void }) {
  const [pkgs, setPkgs] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const { jobs, onEvent, track } = useJobs();

  const load = useCallback(() => {
    devicePackages(device.route_id).then(setPkgs, (e) => setError(errorText(e)));
  }, [device.route_id]);

  useEffect(() => {
    load();
  }, [load]);

  const install = async () => {
    const picked = await open({ title: "Choose APK", multiple: false, filters: [{ name: "Android package", extensions: ["apk"] }] });
    if (typeof picked !== "string") return;
    const name = picked.split(/[\\/]/).pop() ?? picked;
    ask({
      title: "Install APK on this device?",
      body: `${name} → ${device.display_name} (${routeText(device)})`,
      label: "Install",
      run: async () => {
        try {
          const label = `Install ${name}`;
          const handler = onEvent(label);
          const id = await deviceInstallApk(device.route_id, device.serial, picked, (ev) => {
            handler(ev);
            if (ev.state === "done") load();
          });
          track(id, label);
        } catch (e) {
          setError(errorText(e));
        }
      },
    });
  };

  const uninstall = (pkg: string) =>
    ask({
      title: `Uninstall ${pkg}?`,
      body: `From ${device.display_name} (${routeText(device)}). App data on the phone is removed.`,
      label: "Uninstall",
      danger: true,
      run: async () => {
        try {
          setNote(await deviceUninstall(device.route_id, device.serial, pkg));
          load();
        } catch (e) {
          setError(errorText(e));
        }
      },
    });

  return (
    <div className="section">
      <div className="row">
        <button className="primary" onClick={install}>
          Install APK…
        </button>
        <button onClick={load}>Refresh</button>
        <span className="muted small">User-installed apps only.</span>
      </div>
      {error && <p className="banner bad small">{error}</p>}
      {note && <p className="muted small">{note}</p>}
      <Jobs jobs={jobs} />
      {pkgs && (
        <div className="filelist">
          <table className="grid full">
            <tbody>
              {pkgs.length === 0 && (
                <tr>
                  <td className="muted">No user-installed apps</td>
                </tr>
              )}
              {pkgs.map((p) => (
                <tr key={p}>
                  <td>
                    <code>{p}</code>
                  </td>
                  <td className="row-actions">
                    <button className="danger" onClick={() => uninstall(p)}>
                      Uninstall
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

const join = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);
const parentOf = (p: string) => (p === "/" ? "/" : p.replace(/\/[^/]+\/?$/, "") || "/");

function FilesSection({ device, ask }: { device: DeviceView; ask: (c: Confirm) => void }) {
  const [path, setPath] = useState("/sdcard/");
  const [entries, setEntries] = useState<DeviceEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { jobs, onEvent, track } = useJobs();

  const load = useCallback(
    (p: string) => {
      setError(null);
      deviceFilesList(device.route_id, p).then(
        (e) => {
          setEntries(e);
          setPath(p);
        },
        (e) => setError(errorText(e)),
      );
    },
    [device.route_id],
  );

  useEffect(() => {
    deviceFilesList(device.route_id, "/sdcard/").then(setEntries, (e) => setError(errorText(e)));
  }, [device.route_id]);

  const pull = async (en: DeviceEntry, overwrite = false) => {
    const dir = await open({ title: `Save ${en.name} to…`, directory: true, multiple: false });
    if (typeof dir !== "string") return;
    const go = async (ow: boolean) => {
      try {
        const label = `↓ ${en.name}`;
        track(await devicePull(device.route_id, join(path, en.name), dir, ow, onEvent(label)), label);
      } catch (e) {
        if (errorCode(e) === "CONFLICT" && !ow) {
          ask({ title: "Replace existing file?", body: `${en.name} already exists in ${dir}.`, label: "Replace", danger: true, run: () => void go(true) });
        } else setError(errorText(e));
      }
    };
    await go(overwrite);
  };

  const push = async () => {
    const picked = await open({ title: "Copy file to device", multiple: false });
    if (typeof picked !== "string") return;
    const name = picked.split(/[\\/]/).pop() ?? picked;
    const go = async (ow: boolean) => {
      try {
        const label = `↑ ${name}`;
        track(
          await devicePush(device.route_id, device.serial, picked, path, ow, (ev) => {
            onEvent(label)(ev);
            if (ev.state === "done") load(path);
          }),
          label,
        );
      } catch (e) {
        if (errorCode(e) === "CONFLICT" && !ow) {
          ask({ title: "Replace file on device?", body: `${name} already exists in ${path} on ${device.display_name}.`, label: "Replace", danger: true, run: () => void go(true) });
        } else setError(errorText(e));
      }
    };
    ask({ title: "Copy file to this device?", body: `${name} → ${path} on ${device.display_name} (${routeText(device)})`, label: "Copy", run: () => void go(false) });
  };

  return (
    <div className="section">
      <form
        className="pathbar"
        onSubmit={(e) => {
          e.preventDefault();
          load(path);
        }}
      >
        <button type="button" onClick={() => load(parentOf(path))} disabled={path === "/"}>
          ↑ Up
        </button>
        <input value={path} onChange={(e) => setPath(e.target.value)} aria-label="Device path" spellCheck={false} />
        <button type="submit">Go</button>
        <button type="button" className="primary" onClick={push}>
          Upload…
        </button>
      </form>
      {error && <p className="banner bad small">{error}</p>}
      <Jobs jobs={jobs} />
      {entries && (
        <div className="filelist">
          <table className="grid full">
            <tbody>
              {entries.length === 0 && (
                <tr>
                  <td className="muted">Empty folder (or not readable over ADB)</td>
                </tr>
              )}
              {entries.map((en) => (
                <tr key={en.name}>
                  <td>
                    {en.is_dir ? (
                      <button className="link" onClick={() => load(join(path, en.name) + "/")}>
                        📁 {en.name}
                      </button>
                    ) : (
                      <span>📄 {en.name}</span>
                    )}
                  </td>
                  <td className="num">{en.is_dir ? "" : fmtSize(en.size)}</td>
                  <td className="muted small">{en.mtime ? new Date(en.mtime * 1000).toLocaleString() : ""}</td>
                  <td className="row-actions">{!en.is_dir && <button onClick={() => pull(en)}>Download</button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/** One device: screenshot, logcat, apps and files (AND-07, FILE-02). */
export function DeviceDetail() {
  const { deviceRouteId, setView } = useUi();
  const [device, setDevice] = useState<DeviceView | null>(null);
  const [section, setSection] = useState<Section>("screen");
  const [confirm, setConfirm] = useState<Confirm | null>(null);

  useEffect(() => {
    const load = () => devicesList().then((all) => setDevice(all.find((d) => d.route_id === deviceRouteId) ?? null));
    void load();
    const un = listen("devices-changed", () => void load());
    return () => void un.then((u) => u());
  }, [deviceRouteId]);

  if (!device) return <section className="view padded"><p>Loading device…</p></section>;
  const available = device.state === "available";

  return (
    <section className="view padded">
      <header className="view-header">
        <div>
          <button className="link" onClick={() => setView("devices")}>
            ← Devices
          </button>
          <h1>{device.display_name}</h1>
          <p className="target">{routeText(device)}</p>
        </div>
        <span className={`badge ${available ? "ok" : "warn"}`}>{available ? "● Available" : `! ${device.state}`}</span>
      </header>

      <div className="tabbar inline" role="tablist">
        {(["screen", "logcat", "apps", "files"] as Section[]).map((s) => (
          <button key={s} role="tab" aria-selected={section === s} className={`tab ${section === s ? "active" : ""}`} onClick={() => setSection(s)}>
            {{ screen: "Screen", logcat: "Logcat", apps: "Apps", files: "Files" }[s]}
          </button>
        ))}
      </div>

      {!available ? (
        <p className="banner bad">This device is {device.state} on {device.host_name}. Reconnect it to use these tools.</p>
      ) : (
        <>
          {section === "screen" && <ScreenSection device={device} />}
          {section === "logcat" && <LogcatSection key={device.route_id} device={device} />}
          {section === "apps" && <AppsSection device={device} ask={setConfirm} />}
          {section === "files" && <FilesSection device={device} ask={setConfirm} />}
        </>
      )}

      {confirm && (
        <ConfirmDialog
          title={confirm.title}
          confirmLabel={confirm.label}
          danger={confirm.danger}
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            const c = confirm;
            setConfirm(null);
            c.run();
          }}
        >
          <p className="target">{confirm.body}</p>
        </ConfirmDialog>
      )}
    </section>
  );
}
