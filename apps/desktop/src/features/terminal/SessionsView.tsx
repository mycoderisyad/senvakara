import { useEffect, useState } from "react";
import { wslDistros } from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { TerminalPane } from "./TerminalPane";

/** Terminal sessions: local PowerShell/WSL and remote SSH tabs (TERM-01..03). */
export function SessionsView() {
  const { tabs, activeKey, openTab, closeTab, setActive, markExited, reopenTab, hosts, setView } = useUi();
  const [distros, setDistros] = useState<string[]>([]);

  useEffect(() => {
    wslDistros().then(setDistros).catch(() => setDistros([]));
  }, []);

  const sshHosts = hosts.filter((h) => !h.is_local && h.ssh?.fingerprint);

  return (
    <section className="view sessions">
      <div className="tabbar" role="tablist" aria-label="Sessions">
        {tabs.map((t) => (
          <div
            key={t.key}
            role="tab"
            aria-selected={t.key === activeKey}
            className={`tab ${t.key === activeKey ? "active" : ""}`}
            onClick={() => setActive(t.key)}
          >
            <span className={`dot ${t.exited ? "off" : "on"}`} aria-hidden />
            <span>{t.title}</span>
            {t.exited && <span className="tab-state">exited</span>}
            {t.exited && t.shell.kind === "ssh" && (
              <button
                className="tab-close"
                title={t.shell.tmux ? `Reattach tmux session ${t.shell.tmux}` : "Open a new shell on the host"}
                onClick={(e) => {
                  e.stopPropagation();
                  reopenTab(t.key);
                }}
              >
                ↻
              </button>
            )}
            <button
              className="tab-close"
              aria-label={`Close ${t.title}`}
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.key);
              }}
            >
              ×
            </button>
          </div>
        ))}
        <div className="tab-actions">
          <button onClick={() => openTab("PowerShell", { kind: "power_shell" })}>+ PowerShell</button>
          {distros.map((d) => (
            <button key={d} onClick={() => openTab(`WSL · ${d}`, { kind: "wsl", distro: d })}>
              + WSL {d}
            </button>
          ))}
          {sshHosts.map((h) => (
            <span key={h.id} className="tab-group">
              <button onClick={() => openTab(`SSH · ${h.display_name}`, { kind: "ssh", host_id: h.id, tmux: null })}>+ SSH {h.display_name}</button>
              {h.capabilities?.tmux && (
                <button
                  title="Runs inside tmux: the shell keeps running if the connection drops, and ↻ reattaches it"
                  onClick={() => {
                    const name = `senvakara-${Date.now().toString(36)}`;
                    openTab(`tmux · ${h.display_name}`, { kind: "ssh", host_id: h.id, tmux: name });
                  }}
                >
                  + tmux
                </button>
              )}
            </span>
          ))}
        </div>
      </div>

      <div className="tab-body">
        {tabs.length === 0 && (
          <div className="empty">
            <h2>No open sessions</h2>
            <p>Start a local PowerShell or WSL terminal, or an SSH terminal on a trusted host.</p>
            <button onClick={() => setView("hosts")}>Manage hosts</button>
          </div>
        )}
        {tabs.map((t) => (
          <TerminalPane key={t.key} shell={t.shell} active={t.key === activeKey} onExit={() => markExited(t.key)} />
        ))}
      </div>
    </section>
  );
}
