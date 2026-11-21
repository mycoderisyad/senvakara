import { useEffect, useRef, useState } from "react";
import RFB from "@novnc/novnc";
import { desktopClose, desktopOpen, DesktopOpened, errorText } from "../../lib/ipc";
import { useUi } from "../../lib/store";

type Status = "starting" | "connected" | "ended";

/** Full-page remote desktop opened from Hosts. */
export function DesktopView() {
  const { desktopHostId, setView } = useUi();
  if (!desktopHostId) return null;
  return <DesktopPane hostId={desktopHostId} onClose={() => setView("hosts")} />;
}

/**
 * The host's current graphical session (DESK-01..03). The session starts
 * when the pane mounts and ends when it unmounts.
 */
export function DesktopPane({ hostId, onClose }: { hostId: string; onClose?: () => void }) {
  const { hosts } = useUi();
  const desktopHostId = hostId;
  const host = hosts.find((h) => h.id === desktopHostId);
  const screen = useRef<HTMLDivElement>(null);
  const rfb = useRef<RFB | null>(null);
  const [opened, setOpened] = useState<DesktopOpened | null>(null);
  const [status, setStatus] = useState<Status>("starting");
  const [error, setError] = useState<string | null>(null);
  const [viewOnly, setViewOnly] = useState(false);

  useEffect(() => {
    if (!desktopHostId) return;
    let cancelled = false;
    let sessionId: number | null = null;
    desktopOpen(desktopHostId).then(
      (o) => {
        if (cancelled || !screen.current) {
          void desktopClose(o.id);
          return;
        }
        sessionId = o.id;
        setOpened(o);
        const r = new RFB(screen.current, o.ws_url, { shared: true, wsProtocols: ["binary"] });
        r.scaleViewport = true;
        r.resizeSession = false;
        r.focusOnClick = true;
        // The one-time password never leaves memory and is useless after this session.
        r.addEventListener("credentialsrequired", () => r.sendCredentials({ password: o.password }));
        r.addEventListener("connect", () => {
          setStatus("connected");
          r.focus();
        });
        r.addEventListener("disconnect", (e) => {
          setStatus("ended");
          const clean = (e as CustomEvent<{ clean: boolean }>).detail?.clean;
          if (!clean) setError("connection to the desktop was lost");
        });
        r.addEventListener("securityfailure", (e) => {
          setStatus("ended");
          setError(`security failure: ${(e as CustomEvent<{ reason?: string }>).detail?.reason ?? "unknown"}`);
        });
        rfb.current = r;
      },
      (e) => {
        setStatus("ended");
        setError(errorText(e));
      },
    );
    return () => {
      cancelled = true;
      rfb.current?.disconnect();
      rfb.current = null;
      if (sessionId !== null) void desktopClose(sessionId);
    };
  }, [desktopHostId]);

  useEffect(() => {
    if (rfb.current) rfb.current.viewOnly = viewOnly;
  }, [viewOnly]);

  return (
    <section className="view desktop-view">
      <div className="desktop-bar">
        {onClose && (
          <button className="link" onClick={onClose}>
            ← Hosts
          </button>
        )}
        <strong>{host?.display_name ?? "Host"}</strong>
        <span className="muted small">
          {opened ? `${opened.desktop ?? "desktop"} · display ${opened.display}` : ""} · via SSH tunnel · one-time password
        </span>
        <span className={`badge ${status === "connected" ? "ok" : status === "starting" ? "warn" : "bad"}`}>
          {status === "connected" ? "● connected" : status === "starting" ? "◌ starting…" : "✕ ended"}
        </span>
        <label className="check">
          <input type="checkbox" checked={viewOnly} onChange={(e) => setViewOnly(e.target.checked)} /> View only
        </label>
        {onClose && <button onClick={onClose}>Disconnect</button>}
      </div>
      {error && <p className="banner bad small">{error}</p>}
      <div className="desktop-screen" ref={screen} aria-label={`Remote desktop of ${host?.display_name ?? "host"}`} />
    </section>
  );
}
