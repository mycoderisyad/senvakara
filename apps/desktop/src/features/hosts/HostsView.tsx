import { useEffect, useState } from "react";
import { errorText, HostView, hostConnect, hostDelete, hostDisconnect } from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { HostForm } from "./HostForm";
import { TrustKeyDialog } from "./TrustKeyDialog";

type Busy = Record<string, string | undefined>;

/** Connection state shown with text and an icon, never colour alone (NFR-06). */
function ConnectionBadge({ host, busy, error }: { host: HostView; busy?: string; error?: string }) {
  if (host.is_local) return <span className="badge ok">● This PC</span>;
  if (busy) return <span className="badge warn">◌ {busy}…</span>;
  if (!host.ssh?.fingerprint) return <span className="badge warn">! Host key not trusted</span>;
  if (error) return <span className="badge bad">✕ {error.split(":")[0].toLowerCase().replace(/_/g, " ")}</span>;
  if (host.connected) return <span className="badge ok">● Connected</span>;
  return <span className="badge">○ Not connected</span>;
}

export function HostsView() {
  const { hosts, hostsError, refreshHosts, putHost, openTab, openFiles } = useUi();
  const [editing, setEditing] = useState<HostView | "new" | null>(null);
  const [trusting, setTrusting] = useState<HostView | null>(null);
  const [busy, setBusy] = useState<Busy>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [removing, setRemoving] = useState<HostView | null>(null);

  useEffect(() => {
    void refreshHosts();
  }, [refreshHosts]);

  const run = async (h: HostView, label: string, fn: () => Promise<HostView | void>) => {
    setBusy((b) => ({ ...b, [h.id]: label }));
    setErrors((e) => ({ ...e, [h.id]: "" }));
    try {
      const updated = await fn();
      if (updated) putHost(updated);
    } catch (e) {
      setErrors((x) => ({ ...x, [h.id]: errorText(e) }));
    } finally {
      setBusy((b) => ({ ...b, [h.id]: undefined }));
    }
  };

  const remove = async (h: HostView) => {
    setRemoving(null);
    await run(h, "Removing", async () => {
      await hostDelete(h.id);
      await refreshHosts();
    });
  };

  return (
    <section className="view padded">
      <header className="view-header">
        <div>
          <h1>Hosts</h1>
          <p className="muted">Computers Senvakara can reach. Remote hosts connect over SSH with a key and a verified host key.</p>
        </div>
        <button className="primary" onClick={() => setEditing("new")}>
          + Add host
        </button>
      </header>

      {hostsError && <p className="banner bad">{hostsError}</p>}

      <div className="cards">
        {hosts.map((h) => {
          const caps = h.capabilities ?? {};
          const err = errors[h.id];
          return (
            <article key={h.id} className="card" aria-label={h.display_name}>
              <div className="card-head">
                <div>
                  <h3>{h.display_name}</h3>
                  <p className="muted small">
                    {h.is_local
                      ? "Windows · local"
                      : `${h.ssh?.username}@${h.ssh?.address}:${h.ssh?.port}`}
                    {caps.os_pretty_name ? ` · ${caps.os_pretty_name}` : ""}
                    {caps.arch ? ` · ${caps.arch}` : ""}
                  </p>
                </div>
                <ConnectionBadge host={h} busy={busy[h.id]} error={err} />
              </div>

              {!h.is_local && h.ssh?.fingerprint && (
                <p className="small muted">
                  Host key <code>{h.ssh.fingerprint}</code>
                </p>
              )}
              {!h.is_local && caps.adb_path !== undefined && (
                <p className="small muted">ADB: {caps.adb_path ? <code>{caps.adb_path}</code> : "not installed"}</p>
              )}
              {err && <p className="banner bad small">{err}</p>}

              <div className="card-actions">
                {h.is_local ? (
                  <button onClick={() => openTab("PowerShell", { kind: "power_shell" })}>Terminal</button>
                ) : !h.ssh?.fingerprint ? (
                  <button className="primary" onClick={() => setTrusting(h)}>
                    Verify host key
                  </button>
                ) : (
                  <>
                    <button onClick={() => openTab(`SSH · ${h.display_name}`, { kind: "ssh", host_id: h.id })}>Terminal</button>
                    <button onClick={() => openFiles(h.id)}>Files</button>
                    {h.connected ? (
                      <button onClick={() => run(h, "Disconnecting", () => hostDisconnect(h.id))}>Disconnect</button>
                    ) : (
                      <button onClick={() => run(h, "Connecting", () => hostConnect(h.id))}>Connect</button>
                    )}
                    <button onClick={() => setTrusting(h)}>Check host key</button>
                  </>
                )}
                {!h.is_local && (
                  <>
                    <button onClick={() => setEditing(h)}>Edit</button>
                    <button className="danger" onClick={() => setRemoving(h)}>
                      Remove
                    </button>
                  </>
                )}
              </div>
            </article>
          );
        })}
      </div>

      {editing && (
        <HostForm
          host={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async (h) => {
            setEditing(null);
            await refreshHosts();
            if (!h.ssh?.fingerprint) setTrusting(h);
          }}
        />
      )}
      {removing && (
        <ConfirmDialog
          title={`Remove ${removing.display_name}?`}
          confirmLabel="Remove host"
          danger
          onCancel={() => setRemoving(null)}
          onConfirm={() => remove(removing)}
        >
          <p>Its saved settings, trusted host key and stored passphrase are deleted from this PC. Nothing on the host itself is changed.</p>
        </ConfirmDialog>
      )}
      {trusting && (
        <TrustKeyDialog
          host={trusting}
          onClose={() => setTrusting(null)}
          onTrusted={(h) => {
            putHost(h);
            setTrusting(null);
          }}
        />
      )}
    </section>
  );
}
