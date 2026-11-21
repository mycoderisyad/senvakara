import { useState } from "react";
import { SessionsView } from "./features/terminal/SessionsView";
import { DiagnosticsView } from "./features/diagnostics/DiagnosticsView";

type ViewId = "overview" | "hosts" | "devices" | "sessions" | "files" | "workspaces" | "diagnostics";

const NAV: Array<{ id: ViewId; label: string; milestone?: string }> = [
  { id: "overview", label: "Overview", milestone: "M1" },
  { id: "hosts", label: "Hosts", milestone: "M1" },
  { id: "devices", label: "Devices", milestone: "M2" },
  { id: "sessions", label: "Sessions" },
  { id: "files", label: "Files", milestone: "M1" },
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
  const [view, setView] = useState<ViewId>("sessions");
  const current = NAV.find((n) => n.id === view)!;

  return (
    <div className="shell">
      <nav className="sidebar" aria-label="Main">
        <div className="brand">Senvakara</div>
        {NAV.map((n) => (
          <button
            key={n.id}
            className={`nav-item ${n.id === view ? "active" : ""}`}
            aria-current={n.id === view ? "page" : undefined}
            onClick={() => setView(n.id)}
          >
            {n.label}
            {n.milestone && <span className="nav-tag">{n.milestone}</span>}
          </button>
        ))}
      </nav>
      <main className="content">
        {/* Sessions stays mounted so terminals survive view switches. */}
        <div className="view-slot" hidden={view !== "sessions"}>
          <SessionsView />
        </div>
        {view === "diagnostics" && <DiagnosticsView />}
        {view !== "sessions" && view !== "diagnostics" && (
          <NotYet label={current.label} milestone={current.milestone} />
        )}
      </main>
    </div>
  );
}
