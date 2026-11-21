import { ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  errorCode,
  errorText,
  Listing,
  RemoteEntry,
  sftpDownload,
  sftpList,
  sftpUpload,
  transferCancel,
  TransferEvent,
} from "../../lib/ipc";
import { useUi } from "../../lib/store";
import { ConfirmDialog } from "../../components/ConfirmDialog";

interface Job {
  id: number;
  label: string;
  direction: "download" | "upload";
  event: TransferEvent;
}

interface PendingOverwrite {
  label: string;
  retry: () => void;
}

const fmtSize = (n: number) =>
  n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;

const fmtTime = (t: number | null) => (t ? new Date(t * 1000).toLocaleString() : "");

const parentOf = (p: string) => (p === "/" ? "/" : p.replace(/\/[^/]+\/?$/, "") || "/");

const join = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);

/** Remote file browser over SFTP with background transfers (FILE-01). */
export function FilesView() {
  const { hosts, filesHostId, openFiles, setView } = useUi();
  const remoteHosts = useMemo(() => hosts.filter((h) => !h.is_local && h.ssh?.fingerprint), [hosts]);
  const hostId = filesHostId && remoteHosts.some((h) => h.id === filesHostId) ? filesHostId : (remoteHosts[0]?.id ?? null);

  if (!hostId) {
    return (
      <section className="view padded">
        <div className="empty">
          <h2>No trusted remote hosts yet</h2>
          <p>Add a Linux host and trust its host key to browse its files.</p>
          <button className="primary" onClick={() => setView("hosts")}>
            Go to Hosts
          </button>
        </div>
      </section>
    );
  }

  const picker = (
    <label className="inline">
      Host{" "}
      <select value={hostId} onChange={(e) => openFiles(e.target.value)}>
        {remoteHosts.map((h) => (
          <option key={h.id} value={h.id}>
            {h.display_name}
          </option>
        ))}
      </select>
    </label>
  );
  // Keyed by host so all browsing state resets when the host changes.
  return <RemoteBrowser key={hostId} hostId={hostId} hostName={remoteHosts.find((h) => h.id === hostId)?.display_name ?? ""} picker={picker} />;
}

export function RemoteBrowser({ hostId, hostName, picker }: { hostId: string; hostName: string; picker: ReactNode }) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [pathInput, setPathInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [pending, setPending] = useState<PendingOverwrite | null>(null);

  const show = useCallback((l: Listing) => {
    setListing(l);
    setPathInput(l.path);
  }, []);

  const load = useCallback(
    async (path: string) => {
      setLoading(true);
      setError(null);
      try {
        show(await sftpList(hostId, path));
      } catch (e) {
        setError(errorText(e));
      } finally {
        setLoading(false);
      }
    },
    [hostId, show],
  );

  useEffect(() => {
    sftpList(hostId, ".")
      .then(show, (e) => setError(errorText(e)))
      .finally(() => setLoading(false));
  }, [hostId, show]);

  /** Add a job only if no event for it has arrived yet (fast transfers can finish first). */
  const track = (id: number, label: string, direction: Job["direction"], first: TransferEvent) =>
    setJobs((j) => (j.some((x) => x.id === id) ? j : [{ id, label, direction, event: first }, ...j]));

  const onEvent =
    (label: string, direction: Job["direction"], refresh: boolean, conflict: PendingOverwrite | null) =>
    (ev: TransferEvent) => {
      setJobs((j) =>
        j.some((x) => x.id === ev.id)
          ? j.map((x) => (x.id === ev.id ? { ...x, event: ev } : x))
          : [{ id: ev.id, label, direction, event: ev }, ...j],
      );
      // Existing destination: ask before replacing, then retry with overwrite.
      if (ev.state === "failed" && ev.error?.code === "CONFLICT" && conflict) {
        setJobs((j) => j.filter((x) => x.id !== ev.id));
        setPending(conflict);
      }
      if (refresh && ev.state === "done" && listing) void load(listing.path);
    };

  const startDownload = async (entry: RemoteEntry, localDir: string, overwrite: boolean) => {
    if (!listing) return;
    const remote = join(listing.path, entry.name);
    const label = `↓ ${entry.name}`;
    const conflict = overwrite
      ? null
      : { label: `${entry.name} already exists in ${localDir}`, retry: () => void startDownload(entry, localDir, true) };
    try {
      const id = await sftpDownload(hostId, remote, localDir, overwrite, onEvent(label, "download", false, conflict));
      track(id, label, "download", { id, state: "running", done: 0, total: entry.size, result_path: null, error: null });
    } catch (e) {
      if (errorCode(e) === "CONFLICT" && conflict) setPending(conflict);
      else setError(errorText(e));
    }
  };

  const download = async (entry: RemoteEntry) => {
    const dir = await open({ title: `Save ${entry.name} to…`, directory: true, multiple: false });
    if (typeof dir === "string") await startDownload(entry, dir, false);
  };

  const startUpload = async (localPath: string, overwrite: boolean) => {
    if (!listing) return;
    const name = localPath.split(/[\\/]/).pop() ?? localPath;
    const label = `↑ ${name}`;
    const conflict = overwrite
      ? null
      : { label: `${name} already exists in ${listing.path}`, retry: () => void startUpload(localPath, true) };
    try {
      const id = await sftpUpload(hostId, localPath, listing.path, overwrite, onEvent(label, "upload", true, conflict));
      track(id, label, "upload", { id, state: "running", done: 0, total: 0, result_path: null, error: null });
    } catch (e) {
      if (errorCode(e) === "CONFLICT" && conflict) setPending(conflict);
      else setError(errorText(e));
    }
  };

  const upload = async () => {
    const picked = await open({ title: "Upload files", multiple: true, directory: false });
    const files = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    for (const f of files) await startUpload(f, false);
  };

  return (
    <section className="view padded files">
      <header className="view-header">
        <div>
          <h1>Files</h1>
          <p className="muted">Browse and transfer files over SFTP.</p>
        </div>
        {picker}
      </header>

      <form
        className="pathbar"
        onSubmit={(e) => {
          e.preventDefault();
          void load(pathInput);
        }}
      >
        <button type="button" onClick={() => listing && load(parentOf(listing.path))} disabled={!listing || listing.path === "/"} aria-label="Parent folder">
          ↑ Up
        </button>
        <input value={pathInput} onChange={(e) => setPathInput(e.target.value)} aria-label="Remote path" spellCheck={false} />
        <button type="submit">Go</button>
        <button type="button" onClick={() => listing && load(listing.path)} disabled={!listing}>
          Refresh
        </button>
        <button type="button" className="primary" onClick={upload} disabled={!listing}>
          Upload…
        </button>
      </form>

      {error && <p className="banner bad small">{error}</p>}
      {loading && <p className="muted small">Loading {hostName}…</p>}

      {listing && (
        <div className="filelist">
          <table className="grid full">
            <thead>
              <tr>
                <th>Name</th>
                <th className="num">Size</th>
                <th>Modified</th>
                <th aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {listing.entries.length === 0 && (
                <tr>
                  <td colSpan={4} className="muted">
                    Empty folder
                  </td>
                </tr>
              )}
              {listing.entries.map((en) => (
                <tr key={en.name} onDoubleClick={() => en.is_dir && load(join(listing.path, en.name))}>
                  <td>
                    {en.is_dir ? (
                      <button className="link" onClick={() => load(join(listing.path, en.name))}>
                        📁 {en.name}
                      </button>
                    ) : (
                      <span>
                        {en.is_symlink ? "↪ " : "📄 "}
                        {en.name}
                      </span>
                    )}
                  </td>
                  <td className="num">{en.is_dir ? "" : fmtSize(en.size)}</td>
                  <td className="muted small">{fmtTime(en.modified)}</td>
                  <td className="row-actions">{!en.is_dir && <button onClick={() => download(en)}>Download</button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {jobs.length > 0 && (
        <aside className="activity" aria-label="Transfers">
          <h2>Transfers</h2>
          {jobs.map((j) => {
            const { state, done, total, error: err, result_path } = j.event;
            const pct = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : state === "done" ? 100 : 0;
            return (
              <div key={j.id} className="job">
                <div className="job-head">
                  <span>{j.label}</span>
                  <span className={`badge ${state === "done" ? "ok" : state === "running" ? "" : "bad"}`}>
                    {state === "running" ? `${pct}%` : state}
                  </span>
                  {state === "running" && <button onClick={() => transferCancel(j.id)}>Cancel</button>}
                </div>
                <progress max={100} value={pct} aria-label={`${j.label} progress`} />
                {state === "running" && total > 0 && (
                  <p className="muted small">
                    {fmtSize(done)} of {fmtSize(total)}
                  </p>
                )}
                {result_path && <p className="muted small">Saved to {result_path}</p>}
                {err && state !== "cancelled" && <p className="banner bad small">{errorText(err)}</p>}
              </div>
            );
          })}
        </aside>
      )}

      {pending && (
        <ConfirmDialog
          title="Replace existing file?"
          confirmLabel="Replace"
          danger
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const p = pending;
            setPending(null);
            p.retry();
          }}
        >
          <p>{pending.label}. Replacing it cannot be undone.</p>
        </ConfirmDialog>
      )}
    </section>
  );
}
