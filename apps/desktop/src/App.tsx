import { useEffect } from "react";
import { SessionsView } from "./features/terminal/SessionsView";
import { DiagnosticsView } from "./features/diagnostics/DiagnosticsView";
import { HostsView } from "./features/hosts/HostsView";
import { FilesView } from "./features/files/FilesView";
import { DevicesView } from "./features/devices/DevicesView";
import { DeviceDetail } from "./features/devices/DeviceDetail";
import { OverviewView } from "./features/overview/OverviewView";
import { DesktopView } from "./features/desktop/DesktopView";
import { useUi, ViewId } from "./lib/store";

const NAV: Array<{ id: ViewId; label: string; milestone?: string }> = [
  { id: "overview", label: "Overview" },
  { id: "hosts", label: "Hosts" },
  { id: "devices", label: "Devices" },
  { id: "sessions", label: "Sessions" },
  { id: "files", label: "Files" },
  { id: "workspaces", label: "Workspaces", milestone: "M5" },
  { id: "diagnostics", label: "Diagnostics" },
];

function NotYet({ label, milestone }: { label: string; milestone?: string }) {
  return (
    <section className="view padded">
      <div className="empty">
        <h2>{label}</h2>
        <p>Not built yet — planned for milestone {milestone}. See docs/ROADMAP.md.</p>
      </div>
    </section>
  );
}

export default function App() {
  const { view, setView, tabs, refreshHosts } = useUi();
  const navId: ViewId = view === "device" ? "devices" : view === "desktop" ? "hosts" : view;
  const current = NAV.find((n) => n.id === navId)!;

  useEffect(() => {
    void refreshHosts();
  }, [refreshHosts]);

  return (
    <div className="shell">
      <nav className="sidebar" aria-label="Main">
        <div className="brand">Senvakara</div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={`nav-item ${n.id === navId ? "active" : ""}`}
            aria-current={n.id === navId ? "page" : undefined}
            onClick={() => setView(n.id)}
          >
            {n.label}
            {n.id === "sessions" && tabs.length > 0 && <span className="nav-count">{tabs.length}</span>}
            {n.milestone && <span className="nav-tag">{n.milestone}</span>}
          </button>
        ))}
      </nav>
      <main className="content">
        {/* Sessions stays mounted so terminals survive view switches. */}
        <div className="view-slot" hidden={view !== "sessions"}>
          <SessionsView />
        </div>
        {view === "overview" && <OverviewView />}
        {view === "hosts" && <HostsView />}
        {view === "files" && <FilesView />}
        {view === "devices" && <DevicesView />}
        {view === "device" && <DeviceDetail />}
        {view === "desktop" && <DesktopView />}
        {view === "diagnostics" && <DiagnosticsView />}
        {current.milestone && <NotYet label={current.label} milestone={current.milestone} />}
      </main>
    </div>
  );
}
