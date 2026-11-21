import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  devicesList,
  DeviceView,
  errorText,
  hostConnect,
  HostView,
  LayoutKind,
  PanelSpec,
  workspaceDelete,
  WorkspaceRecord,
  workspacesList,
  workspaceSave,
  wslDistros,
} from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { TerminalPane } from "../terminal/TerminalPane";
import { MirrorPanel } from "../devices/MirrorPanel";
import { LogcatSection } from "../devices/DeviceDetail";
import { DesktopPane } from "../desktop/DesktopView";
import { RemoteBrowser } from "../files/FilesView";
import { routeText } from "../devices/DevicesView";

const SLOTS: Record<LayoutKind, number> = { single: 1, columns: 2, grid: 4 };
const LAYOUT_LABEL: Record<LayoutKind, string> = { single: "1 panel", columns: "2 columns", grid: "2 × 2" };

type Slot = PanelSpec | null;

interface Draft {
  id: string | null;
  name: string;
  layout: LayoutKind;
  slots: Slot[];
}

const emptyDraft = (): Draft => ({ id: null, name: "New workspace", layout: "columns", slots: [null, null] });

const panelTitle = (p: PanelSpec, hosts: HostView[], devices: DeviceView[]) => {
  const host = (id: string) => hosts.find((h) => h.id === id)?.display_name ?? "unknown host";
  const dev = (id: string) => devices.find((d) => d.route_id === id);
  switch (p.panel_type) {
    case "terminal":
      return p.target.title;
    case "desktop":
      return `Desktop · ${host(p.target.host_id)}`;
    case "files":
      return `Files · ${host(p.target.host_id)}`;
    case "mirror":
    case "logcat": {
      const d = dev(p.target.route_id);
      return `${p.panel_type === "mirror" ? "Screen" : "Logcat"} · ${d ? `${d.display_name} (${d.host_name})` : "unknown device"}`;
    }
  }
};

/** Why a saved panel cannot run right now, or `null` when it can (WORK-02). */
function unavailableReason(p: PanelSpec, hosts: HostView[], devices: DeviceView[]): { text: string; hostId?: string } | null {
  const hostNeeded = (id: string) => {
    const h = hosts.find((x) => x.id === id);
    if (!h) return { text: "This host was removed from Senvakara." };
    if (!h.is_local && !h.connected) return { text: `${h.display_name} is not connected.`, hostId: h.id };
    return null;
  };
  switch (p.panel_type) {
    case "terminal":
      return p.target.shell.kind === "ssh" ? hostNeeded(p.target.shell.host_id) : null;
    case "desktop": {
      const r = hostNeeded(p.target.host_id);
      if (r) return r;
      const caps = hosts.find((h) => h.id === p.target.host_id)?.capabilities?.desktop;
      if (caps && caps.session !== "x11") return { text: "The host has no X11 session right now (text mode or Wayland)." };
      return null;
    }
    case "files":
      return hostNeeded(p.target.host_id);
    case "mirror":
    case "logcat": {
      const d = devices.find((x) => x.route_id === p.target.route_id);
      if (!d) return { text: "This device was forgotten." };
      if (d.state === "available") return null;
      const h = hosts.find((x) => x.id === d.host_id);
      return {
        text: `${d.display_name} (${routeText(d)}) is ${d.state === "unauthorized" ? "waiting for USB debugging approval" : "not connected"}.`,
        hostId: h && !h.is_local && !h.connected ? h.id : undefined,
      };
    }
  }
}

function PanelBody({ p, devices }: { p: PanelSpec; devices: DeviceView[] }) {
  switch (p.panel_type) {
    case "terminal":
      return <TerminalPane shell={p.target.shell} active onExit={() => {}} />;
    case "desktop":
      return <DesktopPane hostId={p.target.host_id} />;
    case "files":
      return <RemoteBrowser hostId={p.target.host_id} hostName="" picker={null} />;
    case "mirror":
    case "logcat": {
      const d = devices.find((x) => x.route_id === p.target.route_id)!;
      return p.panel_type === "mirror" ? <MirrorPanel device={d} /> : <LogcatSection device={d} />;
    }
  }
}

function AddPanel({ hosts, devices, distros, onAdd }: { hosts: HostView[]; devices: DeviceView[]; distros: string[]; onAdd: (p: PanelSpec) => void }) {
  const remote = hosts.filter((h) => !h.is_local && h.ssh?.fingerprint);
  const options: Array<{ label: string; spec: () => PanelSpec }> = [
    { label: "Terminal · PowerShell", spec: () => ({ panel_type: "terminal", target: { title: "PowerShell", shell: { kind: "power_shell" } } }) },
    ...distros.map((d) => ({ label: `Terminal · WSL ${d}`, spec: (): PanelSpec => ({ panel_type: "terminal", target: { title: `WSL · ${d}`, shell: { kind: "wsl", distro: d } } }) })),
    ...remote.flatMap((h) => [
      { label: `Terminal · SSH ${h.display_name}`, spec: (): PanelSpec => ({ panel_type: "terminal", target: { title: `SSH · ${h.display_name}`, shell: { kind: "ssh", host_id: h.id, tmux: null } } }) },
      ...(h.capabilities?.tmux
        ? [{ label: `Terminal · tmux ${h.display_name} (survives disconnects)`, spec: (): PanelSpec => ({ panel_type: "terminal", target: { title: `tmux · ${h.display_name}`, shell: { kind: "ssh", host_id: h.id, tmux: `senvakara-${Date.now().toString(36)}` } } }) }]
        : []),
      { label: `Files · ${h.display_name}`, spec: (): PanelSpec => ({ panel_type: "files", target: { host_id: h.id } }) },
      { label: `Desktop · ${h.display_name}`, spec: (): PanelSpec => ({ panel_type: "desktop", target: { host_id: h.id } }) },
    ]),
    ...devices.flatMap((d) => [
      { label: `Screen · ${d.display_name} (${routeText(d)})`, spec: (): PanelSpec => ({ panel_type: "mirror", target: { route_id: d.route_id } }) },
      { label: `Logcat · ${d.display_name} (${routeText(d)})`, spec: (): PanelSpec => ({ panel_type: "logcat", target: { route_id: d.route_id } }) },
    ]),
  ];
  const [choice, setChoice] = useState(0);
  return (
    <div className="add-panel">
      <p className="muted small">Empty panel</p>
      <div className="row">
        <select value={choice} onChange={(e) => setChoice(Number(e.target.value))} aria-label="Panel content">
          {options.map((o, i) => (
            <option key={o.label} value={i}>
              {o.label}
            </option>
          ))}
        </select>
        <button className="primary" onClick={() => onAdd(options[choice].spec())}>
          Add
        </button>
      </div>
    </div>
  );
}

/** Saved multi-panel layouts that can be reopened later (WORK-01/02). */
export function WorkspacesView() {
  const { hosts, refreshHosts, putHost } = useUi();
  const [saved, setSaved] = useState<WorkspaceRecord[]>([]);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [distros, setDistros] = useState<string[]>([]);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [connecting, setConnecting] = useState<string | null>(null);

  const reload = useCallback(() => workspacesList().then(setSaved, (e) => setMsg({ ok: false, text: errorText(e) })), []);

  useEffect(() => {
    const t = setTimeout(() => {
      void reload();
      devicesList().then(setDevices, () => {});
      wslDistros().then(setDistros, () => {});
    }, 0);
    const uns = [
      listen("devices-changed", () => devicesList().then(setDevices, () => {})),
      ...["host-lost", "host-reconnected"].map((ev) => listen(ev, () => void refreshHosts())),
    ];
    return () => {
      clearTimeout(t);
      uns.forEach((u) => void u.then((f) => f()));
    };
  }, [reload, refreshHosts]);

  const open = (w: WorkspaceRecord) => {
    const layout = w.layout.layout ?? "columns";
    const slots: Slot[] = Array.from({ length: SLOTS[layout] }, (_, i) => w.panels[i] ?? null);
    setDraft({ id: w.id, name: w.name, layout, slots });
    setMsg(null);
  };

  const setLayout = (layout: LayoutKind) =>
    setDraft((d) => ({ ...d, layout, slots: Array.from({ length: SLOTS[layout] }, (_, i) => d.slots[i] ?? null) }));

  const save = async () => {
    try {
      const w = await workspaceSave({ id: draft.id, name: draft.name, layout: { layout: draft.layout }, panels: draft.slots.filter((s): s is PanelSpec => s !== null) });
      setDraft((d) => ({ ...d, id: w.id }));
      setMsg({ ok: true, text: `Saved “${w.name}”.` });
      void reload();
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    }
  };

  const remove = async () => {
    if (!draft.id) return;
    await workspaceDelete(draft.id).catch((e) => setMsg({ ok: false, text: errorText(e) }));
    setDraft(emptyDraft());
    void reload();
  };

  const connect = async (hostId: string) => {
    setConnecting(hostId);
    try {
      putHost(await hostConnect(hostId));
      setDevices(await devicesList());
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    } finally {
      setConnecting(null);
    }
  };

  const setSlot = (i: number, s: Slot) => setDraft((d) => ({ ...d, slots: d.slots.map((x, j) => (j === i ? s : x)) }));
  const reasons = useMemo(() => draft.slots.map((s) => (s ? unavailableReason(s, hosts, devices) : null)), [draft.slots, hosts, devices]);

  return (
    <section className="view workspace-view">
      <div className="ws-bar">
        <select
          value={draft.id ?? ""}
          onChange={(e) => {
            const w = saved.find((x) => x.id === e.target.value);
            if (w) open(w);
            else setDraft(emptyDraft());
          }}
          aria-label="Saved workspaces"
        >
          <option value="">+ New workspace</option>
          {saved.map((w) => (
            <option key={w.id} value={w.id}>
              {w.name}
            </option>
          ))}
        </select>
        <input className="ws-name" value={draft.name} onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))} aria-label="Workspace name" />
        <div className="seg" role="group" aria-label="Layout">
          {(Object.keys(SLOTS) as LayoutKind[]).map((l) => (
            <button key={l} className={draft.layout === l ? "active" : ""} aria-pressed={draft.layout === l} onClick={() => setLayout(l)}>
              {LAYOUT_LABEL[l]}
            </button>
          ))}
        </div>
        <button className="primary" onClick={save}>
          Save
        </button>
        {draft.id && (
          <button className="danger" onClick={remove}>
            Delete
          </button>
        )}
        {msg && <span className={`small ${msg.ok ? "ok-text" : "bad-text"}`}>{msg.text}</span>}
      </div>

      <div className={`ws-grid ${draft.layout}`}>
        {draft.slots.map((s, i) => (
          <div key={`${draft.id ?? "new"}-${i}`} className="ws-panel">
            {s === null ? (
              <AddPanel hosts={hosts} devices={devices} distros={distros} onAdd={(p) => setSlot(i, p)} />
            ) : (
              <>
                <div className="ws-panel-head">
                  <span>{panelTitle(s, hosts, devices)}</span>
                  <button className="tab-close" aria-label="Remove panel" onClick={() => setSlot(i, null)}>
                    ×
                  </button>
                </div>
                <div className="ws-panel-body">
                  {reasons[i] ? (
                    <div className="empty">
                      <p>{reasons[i]!.text}</p>
                      {reasons[i]!.hostId && (
                        <button onClick={() => connect(reasons[i]!.hostId!)} disabled={connecting !== null}>
                          {connecting ? "Connecting…" : "Connect host"}
                        </button>
                      )}
                    </div>
                  ) : (
                    <PanelBody p={s} devices={devices} />
                  )}
                </div>
              </>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}
