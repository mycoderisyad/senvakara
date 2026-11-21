import { FormEvent, useCallback, useEffect, useState } from "react";
import { errorText, HostView, MdnsService, wirelessConnect, wirelessPair, wirelessScan } from "../../lib/ipc";

interface Props {
  hosts: HostView[];
  watching: (hostId: string) => boolean;
  onClose: () => void;
}

/**
 * Add a phone over Wi-Fi with Android's Wireless debugging (Android 11+):
 * pair once with the code shown on the phone, then connect (AND-05).
 */
export function WirelessDialog({ hosts, watching, onClose }: Props) {
  const usable = hosts.filter((h) => watching(h.id));
  const [hostId, setHostId] = useState(usable[0]?.id ?? "");
  const [found, setFound] = useState<MdnsService[]>([]);
  const [pairAddr, setPairAddr] = useState("");
  const [code, setCode] = useState("");
  const [connectAddr, setConnectAddr] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [log, setLog] = useState<Array<{ ok: boolean; text: string }>>([]);

  const note = (ok: boolean, text: string) => setLog((l) => [...l, { ok, text }]);

  const scan = useCallback(async () => {
    if (!hostId) return;
    try {
      const s = await wirelessScan(hostId);
      setFound(s);
      const p = s.find((x) => x.service.includes("pairing"));
      const c = s.find((x) => x.service.includes("connect"));
      if (p) setPairAddr((v) => v || p.address);
      if (c) setConnectAddr((v) => v || c.address);
    } catch (e) {
      note(false, errorText(e));
    }
  }, [hostId]);

  useEffect(() => {
    const t = setTimeout(() => void scan(), 0);
    return () => clearTimeout(t);
  }, [scan]);

  const pair = async (e: FormEvent) => {
    e.preventDefault();
    setBusy("pair");
    try {
      note(true, await wirelessPair(hostId, pairAddr.trim(), code.trim()));
      setCode("");
      void scan();
    } catch (err) {
      note(false, errorText(err));
    } finally {
      setBusy(null);
    }
  };

  const connect = async (e: FormEvent) => {
    e.preventDefault();
    setBusy("connect");
    try {
      note(true, await wirelessConnect(hostId, connectAddr.trim()));
    } catch (err) {
      note(false, errorText(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal wide" role="dialog" aria-modal="true" aria-labelledby="wifi-title">
        <h2 id="wifi-title">Add a phone over Wi-Fi</h2>
        <p className="muted small">
          On the phone (Android 11+): Developer options → <strong>Wireless debugging</strong> → on. The phone and the chosen host must be on the same network.
          Older phones need a USB cable instead.
        </p>

        <label className="inline">
          Connect through{" "}
          <select value={hostId} onChange={(e) => setHostId(e.target.value)}>
            {usable.map((h) => (
              <option key={h.id} value={h.id}>
                {h.display_name}
                {h.is_local ? " (this PC)" : ""}
              </option>
            ))}
          </select>
          <button type="button" onClick={() => void scan()}>
            Scan
          </button>
        </label>
        {usable.length === 0 && <p className="banner bad small">No host has ADB running. Connect a host first.</p>}

        {found.length > 0 && (
          <div className="found">
            {found.map((s) => (
              <p key={s.service + s.address} className="small">
                <code>{s.address}</code> · {s.service.includes("pairing") ? "pairing screen open" : "ready to connect"} · <span className="muted">{s.name}</span>
              </p>
            ))}
          </div>
        )}

        <form className="step" onSubmit={pair}>
          <h3>1 · Pair (once per phone and host)</h3>
          <p className="muted small">On the phone tap “Pair device with pairing code”, then enter what it shows.</p>
          <div className="row">
            <label className="grow">
              IP address &amp; port
              <input value={pairAddr} onChange={(e) => setPairAddr(e.target.value)} placeholder="192.0.2.20:37123" spellCheck={false} required />
            </label>
            <label className="narrow">
              Code
              <input value={code} onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))} inputMode="numeric" autoComplete="one-time-code" placeholder="123456" required />
            </label>
            <button type="submit" className="primary" disabled={!hostId || busy !== null}>
              {busy === "pair" ? "Pairing…" : "Pair"}
            </button>
          </div>
        </form>

        <form className="step" onSubmit={connect}>
          <h3>2 · Connect</h3>
          <p className="muted small">Use the “IP address &amp; Port” shown at the top of the Wireless debugging screen (different port from pairing).</p>
          <div className="row">
            <label className="grow">
              IP address &amp; port
              <input value={connectAddr} onChange={(e) => setConnectAddr(e.target.value)} placeholder="192.0.2.20:41234" spellCheck={false} required />
            </label>
            <button type="submit" className="primary" disabled={!hostId || busy !== null}>
              {busy === "connect" ? "Connecting…" : "Connect"}
            </button>
          </div>
        </form>

        {log.map((l, i) => (
          <p key={i} className={`banner small ${l.ok ? "ok" : "bad"}`}>
            {l.ok ? "✓ " : "✕ "}
            {l.text}
          </p>
        ))}
        <p className="muted small">A phone added over Wi-Fi is listed as its own device next to its USB entry; Senvakara does not merge them automatically.</p>
        <div className="modal-actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
