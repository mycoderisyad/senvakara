import { KeyboardEvent, PointerEvent, useEffect, useRef, useState, WheelEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { DeviceView, errorText, mirrorInput, MirrorInput, mirrorStart, MirrorStarted, mirrorStop } from "../../lib/ipc";
import { routeText } from "./DevicesView";

/** Fixed pointer id for single-finger input (any id the server has not seen acts as one finger). */
const FINGER = 1;

// Android KeyEvent codes for keys we forward as key events (text goes as text).
const KEYMAP: Record<string, number> = {
  Backspace: 67, Enter: 66, Tab: 61, Escape: 111, Delete: 112,
  ArrowUp: 19, ArrowDown: 20, ArrowLeft: 21, ArrowRight: 22,
  Home: 122, End: 123, PageUp: 92, PageDown: 93,
};

const BUTTONS: Array<{ label: string; title: string; key?: number; input?: MirrorInput }> = [
  { label: "◁", title: "Back", key: 4 },
  { label: "○", title: "Home", key: 3 },
  { label: "▢", title: "Recent apps", key: 187 },
  { label: "⏻", title: "Power", key: 26 },
  { label: "🔉", title: "Volume down", key: 25 },
  { label: "🔊", title: "Volume up", key: 24 },
  { label: "▤", title: "Notifications", input: { kind: "expand_notifications" } },
  { label: "⟳", title: "Rotate", input: { kind: "rotate" } },
];

/**
 * Live screen + input for one device (AND-06). Only runs while shown; the
 * session (and the scrcpy server on the phone) stops when the pane closes.
 */
export function MirrorPanel({ device }: { device: DeviceView }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const session = useRef<MirrorStarted | null>(null);
  const frameSize = useRef({ w: 0, h: 0 });
  const decoder = useRef<VideoDecoder | null>(null);
  const pressed = useRef(false);
  const [info, setInfo] = useState<MirrorStarted | null>(null);
  const [status, setStatus] = useState<"starting" | "live" | "ended">("starting");
  const [error, setError] = useState<string | null>(null);
  const [fps, setFps] = useState(0);
  const frames = useRef(0);

  useEffect(() => {
    let cancelled = false;
    let gotKey = false;
    const ctx = () => canvas.current?.getContext("2d");

    const makeDecoder = () =>
      new VideoDecoder({
        output: (frame) => {
          const c = canvas.current;
          if (c) {
            if (c.width !== frame.displayWidth || c.height !== frame.displayHeight) {
              c.width = frame.displayWidth;
              c.height = frame.displayHeight;
            }
            frameSize.current = { w: frame.displayWidth, h: frame.displayHeight };
            ctx()?.drawImage(frame, 0, 0);
            frames.current++;
          }
          frame.close();
        },
        error: (e) => setError(`decoder: ${e.message}`),
      });

    const onFrame = (buf: ArrayBuffer) => {
      const bytes = new Uint8Array(buf);
      if (bytes[0] === 0) {
        const codec = new TextDecoder().decode(bytes.subarray(1));
        if (decoder.current?.state !== "configured") decoder.current = makeDecoder();
        decoder.current.configure({ codec, optimizeForLatency: true });
        gotKey = false;
        return;
      }
      const dec = decoder.current;
      if (!dec || dec.state !== "configured") return;
      const key = bytes[1] === 1;
      if (!gotKey && !key) return; // wait for a key frame after (re)configure
      gotKey = true;
      const view = new DataView(buf);
      const pts = Number(view.getBigUint64(2));
      dec.decode(new EncodedVideoChunk({ type: key ? "key" : "delta", timestamp: pts, data: bytes.subarray(10) }));
    };

    const un = listen<{ id: number; reason: string }>("mirror-ended", (e) => {
      if (e.payload.id === session.current?.id) {
        setStatus("ended");
        setError(e.payload.reason);
      }
    });

    mirrorStart(device.route_id, onFrame).then(
      (s) => {
        if (cancelled) {
          void mirrorStop(s.id);
          return;
        }
        session.current = s;
        setInfo(s);
        setStatus("live");
        canvas.current?.focus();
      },
      (e) => {
        setStatus("ended");
        setError(errorText(e));
      },
    );

    const tick = setInterval(() => {
      setFps(frames.current);
      frames.current = 0;
    }, 1000);

    return () => {
      cancelled = true;
      clearInterval(tick);
      void un.then((u) => u());
      if (session.current) void mirrorStop(session.current.id);
      session.current = null;
      if (decoder.current && decoder.current.state !== "closed") decoder.current.close();
    };
  }, [device.route_id]);

  // Input must reach the device in order (a gesture is DOWN, MOVE…, UP), but
  // separate invokes may be handled out of order. Chain them, and collapse
  // queued MOVEs to the latest position so a slow link does not lag behind.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const pendingMove = useRef<MirrorInput | null>(null);
  const send = (input: MirrorInput) => {
    const s = session.current;
    if (!s) return;
    const isMove = input.kind === "touch" && input.action === 2;
    if (isMove) {
      const alreadyQueued = pendingMove.current !== null;
      pendingMove.current = input;
      if (alreadyQueued) return;
    }
    queue.current = queue.current
      .then(() => {
        let msg = input;
        if (isMove) {
          msg = pendingMove.current ?? input;
          pendingMove.current = null;
        }
        return mirrorInput(s.id, msg);
      })
      .catch((e) => setError(errorText(e)));
  };

  const point = (e: PointerEvent<HTMLCanvasElement> | WheelEvent<HTMLCanvasElement>) => {
    const c = e.currentTarget;
    const r = c.getBoundingClientRect();
    const { w, h } = frameSize.current;
    return {
      x: Math.round(((e.clientX - r.left) / r.width) * w),
      y: Math.round(((e.clientY - r.top) / r.height) * h),
      width: w,
      height: h,
    };
  };

  // Called only from pointer event handlers (never during render).
  const touch = (action: 0 | 1 | 2, e: PointerEvent<HTMLCanvasElement>) => {
    if (!frameSize.current.w) return;
    if (action === 0) {
      e.currentTarget.setPointerCapture(e.pointerId);
      e.currentTarget.focus();
      pressed.current = true;
    } else if (!pressed.current) return;
    if (action === 1) pressed.current = false;
    send({ kind: "touch", action, pointer_id: FINGER, pressure: action === 1 ? 0 : 1, ...point(e) });
  };

  const key = (code: number) => {
    send({ kind: "key", action: 0, keycode: code, meta_state: 0 });
    send({ kind: "key", action: 1, keycode: code, meta_state: 0 });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLCanvasElement>) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (KEYMAP[e.key] !== undefined) {
      e.preventDefault();
      key(KEYMAP[e.key]);
    } else if (e.key.length === 1) {
      e.preventDefault();
      send({ kind: "text", text: e.key });
    }
  };

  return (
    <div className="mirror">
      <div className="row">
        <span className={`badge ${status === "live" ? "ok" : status === "starting" ? "warn" : "bad"}`}>
          {status === "live" ? `● live · ${fps} fps` : status === "starting" ? "◌ starting…" : "✕ ended"}
        </span>
        <span className="muted small">
          {info ? `${info.device_name} · ${info.width}×${info.height}` : ""} · {routeText(device)}
        </span>
      </div>
      {error && <p className="banner bad small">{error}</p>}
      <div className="mirror-body">
        <canvas
          ref={canvas}
          className="mirror-canvas"
          tabIndex={0}
          aria-label={`Screen of ${device.display_name}. Click to tap, type to send keys.`}
          onPointerDown={(e) => touch(0, e)}
          onPointerMove={(e) => touch(2, e)}
          onPointerUp={(e) => touch(1, e)}
          onPointerCancel={(e) => touch(1, e)}
          onWheel={(e) => frameSize.current.w && send({ kind: "scroll", ...point(e), h: 0, v: Math.max(-1, Math.min(1, -e.deltaY / 100)) })}
          onKeyDown={onKeyDown}
          onContextMenu={(e) => {
            e.preventDefault();
            key(4);
          }}
        />
        <div className="mirror-tools" role="toolbar" aria-label="Device buttons">
          {BUTTONS.map((b) => (
            <button key={b.title} title={b.title} aria-label={b.title} onClick={() => (b.key !== undefined ? key(b.key) : b.input && send(b.input))} disabled={status !== "live"}>
              {b.label}
            </button>
          ))}
        </div>
      </div>
      <p className="muted small">Click/drag = touch · wheel = scroll · right-click = Back · type with the screen focused.</p>
    </div>
  );
}
