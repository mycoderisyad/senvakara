import { FormEvent, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorText, hostCreate, HostInput, hostSetPassphrase, HostView, hostUpdate } from "../../lib/ipc";

interface Props {
  host: HostView | null;
  onClose: () => void;
  onSaved: (h: HostView) => void;
}

/** Add or edit a remote SSH host. Passwords are not supported: key auth only. */
export function HostForm({ host, onClose, onSaved }: Props) {
  const [form, setForm] = useState<HostInput>({
    display_name: host?.display_name ?? "",
    os_family: host?.os_family ?? "linux",
    address: host?.ssh?.address ?? "",
    port: host?.ssh?.port ?? 22,
    username: host?.ssh?.username ?? "",
    private_key_path: host?.ssh?.private_key_path ?? "",
  });
  const [passphrase, setPassphrase] = useState("");
  const [clearPassphrase, setClearPassphrase] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const set = <K extends keyof HostInput>(k: K, v: HostInput[K]) => setForm((f) => ({ ...f, [k]: v }));

  const browseKey = async () => {
    const picked = await open({ title: "Select SSH private key", multiple: false, directory: false });
    if (typeof picked === "string") set("private_key_path", picked);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSaving(true);
    setError(null);
    try {
      let saved = host ? await hostUpdate(host.id, form) : await hostCreate(form);
      if (passphrase) saved = await hostSetPassphrase(saved.id, passphrase);
      else if (clearPassphrase) saved = await hostSetPassphrase(saved.id, null);
      setPassphrase("");
      onSaved(saved);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="modal" role="dialog" aria-modal="true" aria-labelledby="host-form-title" onSubmit={submit}>
        <h2 id="host-form-title">{host ? `Edit ${host.display_name}` : "Add Linux host"}</h2>
        <p className="muted small">
          Senvakara signs in with an SSH key. Add your public key to <code>~/.ssh/authorized_keys</code> on the host first.
        </p>

        <label>
          Name
          <input value={form.display_name} onChange={(e) => set("display_name", e.target.value)} placeholder="Fedora laptop" required autoFocus />
        </label>
        <div className="row">
          <label className="grow">
            Address
            <input value={form.address} onChange={(e) => set("address", e.target.value)} placeholder="192.0.2.10 or host.local" required />
          </label>
          <label className="narrow">
            Port
            <input type="number" min={1} max={65535} value={form.port} onChange={(e) => set("port", Number(e.target.value))} required />
          </label>
        </div>
        <div className="row">
          <label className="grow">
            Username
            <input value={form.username} onChange={(e) => set("username", e.target.value)} autoComplete="off" required />
          </label>
          <label className="narrow">
            OS
            <select value={form.os_family} onChange={(e) => set("os_family", e.target.value)}>
              <option value="linux">Linux</option>
              <option value="unknown">Other</option>
            </select>
          </label>
        </div>
        <label>
          Private key file
          <div className="row">
            <input className="grow" value={form.private_key_path} onChange={(e) => set("private_key_path", e.target.value)} placeholder="C:\Users\you\.ssh\id_ed25519" required />
            <button type="button" onClick={browseKey}>
              Browse…
            </button>
          </div>
        </label>
        <label>
          Key passphrase <span className="muted small">(optional, stored in Windows Credential Manager)</span>
          <input
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            autoComplete="new-password"
            placeholder={host?.ssh?.has_passphrase ? "•••••• saved — type to replace" : "Leave empty if the key has none"}
          />
        </label>
        {host?.ssh?.has_passphrase && (
          <label className="check">
            <input type="checkbox" checked={clearPassphrase} onChange={(e) => setClearPassphrase(e.target.checked)} /> Remove saved passphrase
          </label>
        )}
        {host && <p className="muted small">Changing the address or port requires verifying the host key again.</p>}

        {error && <p className="banner bad small">{error}</p>}
        <div className="modal-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </div>
  );
}
