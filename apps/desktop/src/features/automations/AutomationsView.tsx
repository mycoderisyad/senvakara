import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  devicesList,
  DeviceView,
  errorText,
  FlowEvent,
  flowCancel,
  flowHistory,
  Recipe,
  recipeDelete,
  RecipePermissions,
  recipeRun,
  recipeSave,
  recipesList,
  RecipeStep,
  RunRecord,
  StepKind,
} from "../../lib/ipc";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { routeText } from "../devices/DevicesView";

const KINDS: Array<{ kind: StepKind; label: string; needs?: keyof RecipePermissions }> = [
  { kind: "install_apk", label: "Install APK", needs: "install" },
  { kind: "uninstall_package", label: "Uninstall app", needs: "install" },
  { kind: "launch_app", label: "Launch app" },
  { kind: "wait", label: "Wait" },
  { kind: "device_shell", label: "Shell command", needs: "shell" },
  { kind: "screenshot", label: "Screenshot" },
  { kind: "flutter_deploy", label: "Flutter build & install", needs: "install" },
];
const LABEL = Object.fromEntries(KINDS.map((k) => [k.kind, k.label])) as Record<StepKind, string>;
const ICON: Record<string, string> = { pending: "○", running: "◌", succeeded: "✓", failed: "✕", cancelled: "■", skipped: "–" };
const MAX_LOG = 2000;

const DEFAULTS: Record<StepKind, RecipeStep["params"]> = {
  install_apk: { path: "" },
  uninstall_package: { package: "" },
  launch_app: { package: "" },
  wait: { seconds: 3 },
  device_shell: { command: "" },
  screenshot: { dir: "" },
  flutter_deploy: { project_dir: "", mode: "release", launch: true },
};

/** One-line, human-readable description used in the list and the confirmation. */
export function describeStep(s: RecipeStep): string {
  const p = s.params;
  switch (s.kind) {
    case "install_apk":
      return `Install ${p.path || "(no file)"}`;
    case "uninstall_package":
      return `Uninstall ${p.package || "(no package)"}`;
    case "launch_app":
      return `Launch ${p.package || "(no package)"}`;
    case "wait":
      return `Wait ${p.seconds} s`;
    case "device_shell":
      return `Shell: ${p.command || "(empty)"}`;
    case "screenshot":
      return `Screenshot → ${p.dir || "(no folder)"}`;
    case "flutter_deploy":
      return `Flutter build (${p.mode}) ${p.project_dir || "(no project)"} and install${p.launch ? " + launch" : ""}`;
  }
}

interface Draft {
  id: string | null;
  name: string;
  target_route_id: string;
  steps: RecipeStep[];
  permissions: RecipePermissions;
}

const blank = (routeId: string): Draft => ({
  id: null,
  name: "New recipe",
  target_route_id: routeId,
  steps: [{ kind: "launch_app", params: { ...DEFAULTS.launch_app } }],
  permissions: { install: false, shell: false },
});

interface StepView {
  name: string;
  status: string;
  detail: string | null;
}

/** Saved, permissioned step lists run against one phone (FLOW-04). */
export function AutomationsView() {
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<"run" | "delete" | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [steps, setSteps] = useState<StepView[]>([]);
  const [done, setDone] = useState<{ status: string; message: string } | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [history, setHistory] = useState<RunRecord[]>([]);
  const logBox = useRef<HTMLPreElement>(null);

  const load = useCallback(async () => {
    try {
      const [r, d, h] = await Promise.all([recipesList(), devicesList(), flowHistory()]);
      setRecipes(r);
      setDevices(d);
      setHistory(h.filter((x) => x.recipe_id));
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    const t = setTimeout(() => void load(), 0);
    return () => clearTimeout(t);
  }, [load]);

  useEffect(() => {
    if (logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
  }, [log]);

  const available = devices.filter((d) => d.state === "available");
  const target = useMemo(() => devices.find((d) => d.route_id === draft?.target_route_id), [devices, draft?.target_route_id]);
  const running = runId !== null && done === null;
  const missingPerm = draft?.steps.find((s) => {
    const need = KINDS.find((k) => k.kind === s.kind)?.needs;
    return need && !draft.permissions[need];
  });

  const edit = (f: (d: Draft) => Draft) => {
    setDraft((d) => (d ? f(d) : d));
    setDirty(true);
  };
  const setStep = (i: number, s: RecipeStep) => edit((d) => ({ ...d, steps: d.steps.map((x, j) => (j === i ? s : x)) }));
  const moveStep = (i: number, by: number) =>
    edit((d) => {
      const steps = [...d.steps];
      const [s] = steps.splice(i, 1);
      steps.splice(i + by, 0, s);
      return { ...d, steps };
    });

  /** Forget the last run's progress when another recipe is opened. */
  const clearRun = () => {
    setRunId(null);
    setSteps([]);
    setLog([]);
    setDone(null);
    setError(null);
  };

  const select = (r: Recipe) => {
    setDraft({ id: r.id, name: r.name, target_route_id: r.target_route_id ?? "", steps: r.steps, permissions: r.permissions });
    setDirty(false);
    clearRun();
  };

  const save = async (): Promise<Recipe | null> => {
    if (!draft) return null;
    setError(null);
    try {
      const r = await recipeSave({ ...draft, target_route_id: draft.target_route_id || null });
      setDraft((d) => (d ? { ...d, id: r.id } : d));
      setDirty(false);
      setRecipes(await recipesList());
      return r;
    } catch (e) {
      setError(errorText(e));
      return null;
    }
  };

  const remove = async () => {
    setConfirm(null);
    if (!draft?.id) return;
    try {
      await recipeDelete(draft.id);
      setDraft(null);
      setRecipes(await recipesList());
    } catch (e) {
      setError(errorText(e));
    }
  };

  const run = async () => {
    setConfirm(null);
    if (!draft || !target) return;
    const saved = dirty || !draft.id ? await save() : { id: draft.id };
    if (!saved) return;
    setSteps(draft.steps.map((s) => ({ name: s.kind, status: "pending", detail: null })));
    setLog([]);
    setDone(null);
    setError(null);
    try {
      const id = await recipeRun(saved.id, target.serial, (ev: FlowEvent) => {
        if (ev.kind === "step") setSteps((s) => s.map((x, i) => (i === ev.ordinal ? { name: ev.name, status: ev.status, detail: ev.detail } : x)));
        else if (ev.kind === "log") setLog((l) => (l.length >= MAX_LOG ? l.slice(-MAX_LOG + 1) : l).concat(ev.text));
        else {
          setDone({ status: ev.status, message: ev.message });
          setSteps((s) => s.map((x) => (x.status === "pending" ? { ...x, status: "skipped" } : x)));
          void flowHistory().then((h) => setHistory(h.filter((x) => x.recipe_id)), () => {});
        }
      });
      setRunId(id);
    } catch (e) {
      setRunId(null);
      setSteps([]);
      setError(errorText(e));
    }
  };

  const recipeHistory = history.filter((h) => h.recipe_id === draft?.id);

  return (
    <section className="view padded automations">
      <header className="view-header">
        <div>
          <h1>Automations</h1>
          <p className="muted">Save a sequence of steps for one phone and run it again with a single confirmation.</p>
        </div>
        <button className="primary" disabled={running} onClick={() => {
            setDraft(blank(available[0]?.route_id ?? ""));
            setDirty(true);
            clearRun();
          }}>
          New recipe
        </button>
      </header>

      <div className="auto-layout">
        <ul className="auto-list" aria-label="Recipes">
          {recipes.length === 0 && <li className="muted small">No recipes yet.</li>}
          {recipes.map((r) => (
            <li key={r.id}>
              <button className={`list-item ${draft?.id === r.id ? "active" : ""}`} disabled={running} onClick={() => select(r)}>
                {r.name}
                <span className="muted small"> · {r.steps.length} steps</span>
              </button>
            </li>
          ))}
        </ul>

        {draft ? (
          <div className="auto-editor">
            <label>
              Name
              <input value={draft.name} maxLength={80} onChange={(e) => edit((d) => ({ ...d, name: e.target.value }))} />
            </label>
            <label>
              Target phone
              <select value={draft.target_route_id} onChange={(e) => edit((d) => ({ ...d, target_route_id: e.target.value }))}>
                <option value="">— choose —</option>
                {devices.map((d) => (
                  <option key={d.route_id} value={d.route_id}>
                    {d.display_name} — {routeText(d)}
                    {d.state !== "available" ? ` (${d.state})` : ""}
                  </option>
                ))}
              </select>
            </label>
            <fieldset className="perms">
              <legend>Permissions for this recipe</legend>
              <label className="check">
                <input type="checkbox" checked={draft.permissions.install} onChange={(e) => edit((d) => ({ ...d, permissions: { ...d.permissions, install: e.target.checked } }))} />
                Install / uninstall apps
              </label>
              <label className="check">
                <input type="checkbox" checked={draft.permissions.shell} onChange={(e) => edit((d) => ({ ...d, permissions: { ...d.permissions, shell: e.target.checked } }))} />
                Run shell commands on the phone
              </label>
            </fieldset>

            <ol className="auto-steps" aria-label="Recipe steps">
              {draft.steps.map((s, i) => (
                <li key={i}>
                  <span className="muted small">{i + 1}.</span>
                  <select
                    value={s.kind}
                    aria-label={`Step ${i + 1} type`}
                    onChange={(e) => {
                      const kind = e.target.value as StepKind;
                      setStep(i, { kind, params: { ...DEFAULTS[kind] } });
                    }}
                  >
                    {KINDS.map((k) => (
                      <option key={k.kind} value={k.kind}>
                        {k.label}
                      </option>
                    ))}
                  </select>
                  <StepParams step={s} onChange={(p) => setStep(i, { ...s, params: p })} />
                  <span className="step-tools">
                    <button type="button" title="Move up" aria-label={`Move step ${i + 1} up`} disabled={i === 0} onClick={() => moveStep(i, -1)}>
                      ↑
                    </button>
                    <button type="button" title="Move down" aria-label={`Move step ${i + 1} down`} disabled={i === draft.steps.length - 1} onClick={() => moveStep(i, 1)}>
                      ↓
                    </button>
                    <button type="button" title="Remove" aria-label={`Remove step ${i + 1}`} disabled={draft.steps.length === 1} onClick={() => edit((d) => ({ ...d, steps: d.steps.filter((_, j) => j !== i) }))}>
                      ✕
                    </button>
                  </span>
                </li>
              ))}
            </ol>
            <div className="row">
              <button type="button" disabled={draft.steps.length >= 20} onClick={() => edit((d) => ({ ...d, steps: [...d.steps, { kind: "wait", params: { ...DEFAULTS.wait } }] }))}>
                + Add step
              </button>
              <span className="grow" />
              {draft.id && (
                <button className="danger" disabled={running} onClick={() => setConfirm("delete")}>
                  Delete
                </button>
              )}
              <button disabled={!dirty || running} onClick={() => void save()}>
                Save
              </button>
              {running ? (
                <button className="danger" onClick={() => runId && flowCancel(runId)}>
                  Cancel run
                </button>
              ) : (
                <button className="primary" disabled={!target || target.state !== "available"} onClick={() => setConfirm("run")}>
                  Run…
                </button>
              )}
            </div>
            {missingPerm && (
              <p className="banner small">
                Step “{LABEL[missingPerm.kind]}” needs a permission that is not granted — the run will be refused until you tick it.
              </p>
            )}
            {target && target.state !== "available" && <p className="muted small">The target phone is {target.state}; connect it to run this recipe.</p>}
            {error && <p className="banner bad small">{error}</p>}

            {steps.length > 0 && (
              <div className="deploy-run">
                <ol className="steps" aria-label="Run progress">
                  {steps.map((s, i) => (
                    <li key={i} className={`step-${s.status}`}>
                      <span className="step-icon" aria-hidden>
                        {ICON[s.status] ?? "?"}
                      </span>
                      <span className="step-name">
                        {i + 1}. {LABEL[s.name as StepKind] ?? s.name}
                      </span>
                      <span className="step-status">{s.status}</span>
                      {s.detail && <span className="muted small step-detail">{s.detail}</span>}
                    </li>
                  ))}
                </ol>
                {done && (
                  <p className={`banner small ${done.status === "succeeded" ? "ok" : "bad"}`}>
                    {done.status === "succeeded" ? "✓ " : "✕ "}
                    {done.message}
                  </p>
                )}
                {log.length > 0 && (
                  <pre className="output log" ref={logBox} aria-label="Run log">
                    {log.join("\n")}
                  </pre>
                )}
              </div>
            )}

            {recipeHistory.length > 0 && (
              <>
                <h2>History</h2>
                <HistoryTable runs={recipeHistory} />
              </>
            )}
          </div>
        ) : (
          <div className="auto-editor">
            <p className="muted">Select a recipe or create a new one.</p>
            {history.length > 0 && (
              <>
                <h2>All recipe runs</h2>
                <HistoryTable runs={history} showRecipe />
              </>
            )}
          </div>
        )}
      </div>

      {confirm === "run" && draft && target && (
        <ConfirmDialog title={`Run “${draft.name}”?`} confirmLabel="Run recipe" onCancel={() => setConfirm(null)} onConfirm={() => void run()}>
          <p className="target">
            → {target.display_name} — {routeText(target)}
          </p>
          <ol className="small confirm-steps">
            {draft.steps.map((s, i) => (
              <li key={i}>{describeStep(s)}</li>
            ))}
          </ol>
          <p className="muted small">
            Permissions: {draft.permissions.install ? "install/uninstall" : "no installs"} · {draft.permissions.shell ? "shell commands" : "no shell"}. The run stops at the first failed step.
          </p>
        </ConfirmDialog>
      )}
      {confirm === "delete" && draft && (
        <ConfirmDialog title="Delete recipe?" confirmLabel="Delete" danger onCancel={() => setConfirm(null)} onConfirm={() => void remove()}>
          <p className="small">
            “{draft.name}” will be removed. Its past runs stay in history.
          </p>
        </ConfirmDialog>
      )}
    </section>
  );
}

function HistoryTable({ runs, showRecipe }: { runs: RunRecord[]; showRecipe?: boolean }) {
  return (
    <table className="grid full">
      <thead>
        <tr>
          <th>When</th>
          {showRecipe && <th>Recipe</th>}
          <th>Target</th>
          <th>Result</th>
          <th className="num">Time</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((r) => (
          <tr key={r.id}>
            <td className="small">{new Date(r.created_at).toLocaleString()}</td>
            {showRecipe && <td className="small">{r.summary.recipe}</td>}
            <td className="small">{r.summary.target}</td>
            <td className="small">
              <span className={`badge ${r.status === "succeeded" ? "ok" : r.status === "running" ? "" : "bad"}`}>{r.status}</span> {r.summary.result}
            </td>
            <td className="num small">{r.summary.seconds !== undefined ? `${r.summary.seconds} s` : ""}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function StepParams({ step, onChange }: { step: RecipeStep; onChange: (p: RecipeStep["params"]) => void }) {
  const p = step.params;
  const set = (k: string, v: string | number | boolean) => onChange({ ...p, [k]: v });
  const text = (k: string, placeholder: string, mono = true) => (
    <input className={`grow ${mono ? "mono" : ""}`} value={String(p[k] ?? "")} placeholder={placeholder} spellCheck={false} aria-label={k} onChange={(e) => set(k, e.target.value)} />
  );
  const browse = async (k: string, directory: boolean) => {
    const r = await open(directory ? { directory: true, multiple: false } : { multiple: false, filters: [{ name: "Android package", extensions: ["apk"] }] });
    if (typeof r === "string") set(k, r);
  };
  switch (step.kind) {
    case "install_apk":
      return (
        <span className="params">
          {text("path", "C:\\path\\app.apk")}
          <button type="button" onClick={() => void browse("path", false)}>
            Browse…
          </button>
        </span>
      );
    case "uninstall_package":
    case "launch_app":
      return <span className="params">{text("package", "com.example.app")}</span>;
    case "wait":
      return (
        <span className="params">
          <input type="number" min={1} max={300} value={Number(p.seconds ?? 1)} aria-label="seconds" onChange={(e) => set("seconds", Math.round(Number(e.target.value)))} /> s
        </span>
      );
    case "device_shell":
      return <span className="params">{text("command", "getprop ro.build.version.release")}</span>;
    case "screenshot":
      return (
        <span className="params">
          {text("dir", "Save to folder")}
          <button type="button" onClick={() => void browse("dir", true)}>
            Browse…
          </button>
        </span>
      );
    case "flutter_deploy":
      return (
        <span className="params">
          {text("project_dir", "Flutter project folder")}
          <button type="button" onClick={() => void browse("project_dir", true)}>
            Browse…
          </button>
          <select value={String(p.mode ?? "release")} aria-label="mode" onChange={(e) => set("mode", e.target.value)}>
            <option value="release">release</option>
            <option value="debug">debug</option>
          </select>
          <label className="check">
            <input type="checkbox" checked={Boolean(p.launch)} onChange={(e) => set("launch", e.target.checked)} /> launch
          </label>
        </span>
      );
  }
}
