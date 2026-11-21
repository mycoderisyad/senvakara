import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { listen } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";
import { errorText, ShellRequest, termClose, termOpen, termResize, termWrite } from "../../lib/ipc";

interface Props {
  shell: ShellRequest;
  active: boolean;
  onExit: () => void;
}

/**
 * One interactive terminal bound to one backend PTY session.
 * The session is closed (and its process killed) when the pane unmounts.
 */
export function TerminalPane({ shell, active, onExit }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const onExitRef = useRef(onExit);
  useEffect(() => {
    onExitRef.current = onExit;
  });

  useEffect(() => {
    const el = host.current;
    if (!el) return;

    const term = new Terminal({
      cursorBlink: true,
      fontFamily: '"Cascadia Mono", Consolas, "Courier New", monospace',
      fontSize: 13,
      scrollback: 5000,
      theme: { background: "#0b1016", foreground: "#eaf0f6", cursor: "#69b8a9" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();
    fitRef.current = fit;

    let id: number | null = null;
    let disposed = false;
    const unlisten = listen<{ id: number }>("term-exit", (ev) => {
      if (ev.payload.id === id) {
        term.write("\r\n\x1b[90m[process exited]\x1b[0m\r\n");
        onExitRef.current();
      }
    });

    termOpen(shell, term.cols, term.rows, (bytes) => term.write(bytes))
      .then((sid) => {
        if (disposed) {
          void termClose(sid);
          return;
        }
        id = sid;
        term.focus();
      })
      .catch((e) => term.write(`\x1b[31mFailed to start shell: ${errorText(e)}\x1b[0m\r\n`));

    // xterm also answers ConPTY's cursor-position query through onData.
    const input = term.onData((d) => {
      if (id !== null) void termWrite(id, d);
    });
    const resize = term.onResize(({ cols, rows }) => {
      if (id !== null) void termResize(id, cols, rows);
    });
    const ro = new ResizeObserver(() => {
      if (el.offsetParent !== null) fit.fit();
    });
    ro.observe(el);

    return () => {
      disposed = true;
      ro.disconnect();
      input.dispose();
      resize.dispose();
      void unlisten.then((u) => u());
      if (id !== null) void termClose(id);
      term.dispose();
    };
    // `shell` is the same object for a tab's whole life, so this runs once per pane.
  }, [shell]);

  useEffect(() => {
    if (active) requestAnimationFrame(() => fitRef.current?.fit());
  }, [active]);

  return <div className="terminal-host" ref={host} hidden={!active} />;
}
