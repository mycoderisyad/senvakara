import { useState } from "react";
import { errorText, HostView, hostAddVpn, hostRemoveVpn } from "../../lib/ipc";

/**
 * Addresses one host can be reached at (NET-IMPL-01/03): the main SSH
 * address, then VPN addresses. Senvakara does not create VPNs; every path
 * must present the same trusted host key.
 */
export function NetworkPaths({ host, onChange }: { host: HostView; onChange: (h: HostView) => void }) {
  const [open, setOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [port, setPort] = useState(host.ssh?.port ?? 22);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ssh = host.ssh;
  if (!ssh) return null;

  const known = new Set([ssh.address, ...host.vpn.map((v) => v.address)]);
  const hints = (host.capabilities?.vpn_interfaces ?? []).filter((i) => !known.has(i.address));
  const active = host.connected ? host.path : null;

  const act = async (fn: () => Promise<HostView>) => {
    setBusy(true);
    setError(null);
    try {
      onChange(await fn());
      setAddress("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const mark = (transport: "lan" | "vpn", addr: string, p: number) =>
    active && active.transport === transport && active.address === addr && active.port === p ? <span className="badge ok">● in use</span> : null;

  return (
    <div className="net-paths">
      <button type="button" className="link small" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? "▾" : "▸"} Network paths{host.vpn.length > 0 ? ` · ${host.vpn.length} VPN` : ""}
        {active && ` · via ${active.transport === "vpn" ? "VPN" : "LAN"} ${active.address}`}
      </button>
      {active?.note && <p className="small muted">{active.note}</p>}
      {open && (
        <div className="net-paths-body">
          <ul className="small">
            <li>
              <span className="path-kind">LAN</span> <code>{ssh.address}:{ssh.port}</code> <span className="muted">tried first</span> {mark("lan", ssh.address, ssh.port)}
            </li>
            {host.vpn.map((v) => (
              <li key={v.endpoint_id}>
                <span className="path-kind">VPN</span> <code>{v.address}:{v.port}</code> {mark("vpn", v.address, v.port)}
                <button type="button" className="link bad-text" disabled={busy} aria-label={`Remove VPN address ${v.address}`} onClick={() => act(() => hostRemoveVpn(host.id, v.endpoint_id))}>
                  remove
                </button>
              </li>
            ))}
          </ul>
          {hints.length > 0 && (
            <p className="small">
              Seen on this host:{" "}
              {hints.map((i) => (
                <button key={i.address} type="button" disabled={busy} onClick={() => act(() => hostAddVpn(host.id, i.address, ssh.port))}>
                  + {i.name} {i.address}
                </button>
              ))}
            </p>
          )}
          <form
            className="row"
            onSubmit={(e) => {
              e.preventDefault();
              if (address.trim()) void act(() => hostAddVpn(host.id, address, port));
            }}
          >
            <input className="grow mono" value={address} onChange={(e) => setAddress(e.target.value)} placeholder="VPN address, e.g. 10.8.0.3 or my-host.tailnet" aria-label="VPN address" spellCheck={false} />
            <input type="number" className="port" min={1} max={65535} value={port} onChange={(e) => setPort(Number(e.target.value))} aria-label="VPN SSH port" />
            <button type="submit" disabled={busy || !address.trim()}>
              Add VPN address
            </button>
          </form>
          <p className="small muted">
            Use a VPN you already run (WireGuard, Tailscale, …) on this PC and the host. Senvakara tries the LAN address first and falls back to these when it
            is unreachable; the host must show the same trusted key on every address. ADB is never exposed — Android traffic stays inside the SSH connection.
          </p>
          {error && <p className="banner bad small">{error}</p>}
        </div>
      )}
    </div>
  );
}
