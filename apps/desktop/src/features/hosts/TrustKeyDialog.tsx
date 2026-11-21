import { useEffect, useState } from "react";
import { errorText, hostCheckKey, HostKeyCheck, hostTrustKey, HostView } from "../../lib/ipc";

interface Props {
  host: HostView;
  onClose: () => void;
  onTrusted: (h: HostView) => void;
}

/**
 * Shows the host key the server presents and asks the user to trust it
 * (docs/DESIGN.md PairingDialog, docs/SECURITY.md §3.2). A changed key is
 * shown as a warning; trusting it is an explicit decision.
 */
export function TrustKeyDialog({ host, onClose, onTrusted }: Props) {
  const [check, setCheck] = useState<HostKeyCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    hostCheckKey(host.id).then(setCheck, (e) => setError(errorText(e)));
  }, [host.id]);

  const changed = check && check.pinned && !check.matches;

  const trust = async () => {
    if (!check) return;
    setSaving(true);
    setError(null);
    try {
      onTrusted(await hostTrustKey(host.id, check.presented));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby="trust-title">
        <h2 id="trust-title">Host key for {host.display_name}</h2>
        <p className="muted small">
          {host.ssh?.address}:{host.ssh?.port}
        </p>
        {!check && !error && <p>Contacting the host…</p>}
        {check && (
          <>
            {changed ? (
              <p className="banner bad">
                ✕ The host key has <strong>changed</strong>. This can mean the host was reinstalled, or that another machine is
                impersonating it. Only continue if you know why it changed.
              </p>
            ) : check.matches ? (
              <p className="banner ok">✓ The host presents the key you already trusted.</p>
            ) : (
              <p>
                First connection. Compare this fingerprint with the one on the host before trusting it. On the host, run:{" "}
                <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code>
              </p>
            )}
            <dl className="kv">
              <dt>Presented</dt>
              <dd>
                <code className="fingerprint">{check.presented}</code>
              </dd>
              {check.pinned && (
                <>
                  <dt>Trusted</dt>
                  <dd>
                    <code className="fingerprint">{check.pinned}</code>
                  </dd>
                </>
              )}
            </dl>
          </>
        )}
        {error && <p className="banner bad small">{error}</p>}
        <div className="modal-actions">
          <button onClick={onClose}>{check?.matches ? "Close" : "Cancel"}</button>
          {check && !check.matches && (
            <button className={changed ? "danger" : "primary"} onClick={trust} disabled={saving}>
              {changed ? "Trust new key" : "Trust this key"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
