import { useEffect, useState } from "react";
import { ShellRequest, wslDistros } from "../../lib/ipc";
import { TerminalPane } from "./TerminalPane";

interface Tab {
  key: number;
  title: string;
  shell: ShellRequest;
  exited: boolean;
}

let nextKey = 1;

/** Local terminal sessions: PowerShell and WSL tabs (TERM-01, TERM-03). */
export function SessionsView() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeKey, setActiveKey] = useState<number | null>(null);
  const [distros, setDistros] = useState<string[]>([]);

  useEffect(() => {
    wslDistros().then(setDistros).catch(() => setDistros([]));
  }, []);

  const open = (title: string, shell: ShellRequest) => {
    const tab = { key: nextKey++, title, shell, exited: false };
    setTabs((t) => [...t, tab]);
    setActiveKey(tab.key);
  };

  const close = (key: number) => {
    const rest = tabs.filter((x) => x.key !== key);
    setTabs(rest);
    if (activeKey === key) setActiveKey(rest.at(-1)?.key ?? null);
  };

  const markExited = (key: number) =>
    setTabs((t) => t.map((x) => (x.key === key ? { ...x, exited: true } : x)));

  return (
    <section className="view sessions">
      <div className="tabbar" role="tablist" aria-label="Sessions">
        {tabs.map((t) => (
          <div
            key={t.key}
            role="tab"
            aria-selected={t.key === activeKey}
            className={`tab ${t.key === activeKey ? "active" : ""}`}
            onClick={() => setActiveKey(t.key)}
          >
            <span className={`dot ${t.exited ? "off" : "on"}`} aria-hidden />
            <span>{t.title}</span>
            {t.exited && <span className="tab-state">exited</span>}
            <button
              className="tab-close"
              aria-label={`Close ${t.title}`}
              onClick={(e) => {
                e.stopPropagation();
                close(t.key);
              }}
            >
              ×
            </button>
          </div>
        ))}
        <div className="tab-actions">
          <button onClick={() => open("PowerShell", { kind: "power_shell" })}>+ PowerShell</button>
          {distros.map((d) => (
            <button key={d} onClick={() => open(`WSL · ${d}`, { kind: "wsl", distro: d })}>
              + WSL {d}
            </button>
          ))}
        </div>
      </div>

      <div className="tab-body">
        {tabs.length === 0 && (
          <div className="empty">
            <h2>No open sessions</h2>
            <p>Start a local PowerShell or WSL terminal. SSH sessions arrive with Hosts.</p>
          </div>
        )}
        {tabs.map((t) => (
          <TerminalPane
            key={t.key}
            shell={t.shell}
            active={t.key === activeKey}
            onExit={() => markExited(t.key)}
          />
        ))}
      </div>
    </section>
  );
}
