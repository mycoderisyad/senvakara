import { useEffect, useState } from "react";
import { runWebCodecsProbe, WebCodecsReport } from "./webcodecs";

declare global {
  interface Window {
    __svkWebCodecs?: WebCodecsReport;
  }
}

function Verdict({ value }: { value: boolean | string }) {
  if (value === true) return <span className="badge ok">✓ supported</span>;
  if (value === false) return <span className="badge bad">✕ not supported</span>;
  return <span className="badge warn">! {value}</span>;
}

/** Run the probe and expose the result for automated checks (CDP). */
async function probeAndPublish(): Promise<WebCodecsReport> {
  const r = await runWebCodecsProbe();
  window.__svkWebCodecs = r;
  return r;
}

/** Runtime capability checks for this machine's WebView2. */
export function DiagnosticsView() {
  const [report, setReport] = useState<WebCodecsReport | null>(null);
  const [running, setRunning] = useState(true);

  useEffect(() => {
    probeAndPublish()
      .then(setReport)
      .finally(() => setRunning(false));
  }, []);

  const run = () => {
    setRunning(true);
    probeAndPublish()
      .then(setReport)
      .finally(() => setRunning(false));
  };

  return (
    <section className="view padded">
      <header className="view-header">
        <div>
          <h1>Diagnostics</h1>
          <p className="muted">Checks this computer's embedded browser engine (WebView2) for features later milestones depend on.</p>
        </div>
        <button onClick={run} disabled={running}>
          {running ? "Running…" : "Run again"}
        </button>
      </header>

      <h2>Video decoding (WebCodecs)</h2>
      <p className="muted">Needed to show Android screens inside the app. H.264 is what scrcpy sends by default.</p>
      {!report && <p>Running checks…</p>}
      {report && !report.available && <p className="badge bad">WebCodecs API is not available in this WebView2.</p>}
      {report && report.available && (
        <>
          <table className="grid">
            <thead>
              <tr>
                <th>Codec</th>
                <th>Decode</th>
                <th>Encode</th>
              </tr>
            </thead>
            <tbody>
              {report.probes.map((p) => (
                <tr key={p.codec}>
                  <td>
                    {p.label} <code>{p.codec}</code>
                  </td>
                  <td>
                    <Verdict value={p.decoder} />
                  </td>
                  <td>
                    <Verdict value={p.encoder} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <p>
            H.264 encode → decode round trip:{" "}
            <span className={`badge ${report.h264RoundTrip.ok ? "ok" : "bad"}`}>
              {report.h264RoundTrip.ok ? "✓ passed" : "✕ failed"}
            </span>{" "}
            <span className="muted">{report.h264RoundTrip.detail}</span>
          </p>
        </>
      )}
      {report && <p className="muted small">Engine: {report.userAgent}</p>}
    </section>
  );
}
