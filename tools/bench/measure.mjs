// Whole-process-tree benchmark for the Windows app (task QA-03).
//
// Launches the given exe with the WebView2 debug port, measures time until
// the UI is usable (sidebar rendered), then samples memory and CPU of the
// app process plus every descendant (WebView2 browser, GPU, renderer...).
//
// Usage: node tools/bench/measure.mjs <path-to-exe> [--idle 300] [--runs 3] [--connect]
//   --idle     seconds to stay idle before measuring (spec: 300)
//   --runs     startup runs (each launch is closed before the next)
//   --connect  after the idle measurement, connect the first remote host,
//              open Devices and measure again after 60 s
import { spawn, execFileSync } from "node:child_process";

const args = process.argv.slice(2);
const exe = args[0];
const opt = (name, def) => {
  const i = args.indexOf(name);
  return i < 0 ? def : Number(args[i + 1]);
};
const idle = opt("--idle", 300);
const runs = opt("--runs", 3);
const connect = args.includes("--connect");
const PORT = 9334;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function procs() {
  const json = execFileSync("powershell", [
    "-NoProfile", "-Command",
    "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,WorkingSetSize,PrivatePageCount,KernelModeTime,UserModeTime | ConvertTo-Json -Compress",
  ], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  return JSON.parse(json);
}

function tree(rootPid) {
  const all = procs();
  const kids = new Map();
  for (const p of all) {
    if (!kids.has(p.ParentProcessId)) kids.set(p.ParentProcessId, []);
    kids.get(p.ParentProcessId).push(p);
  }
  const out = [];
  const stack = all.filter((p) => p.ProcessId === rootPid);
  while (stack.length) {
    const p = stack.pop();
    out.push(p);
    for (const c of kids.get(p.ProcessId) ?? []) if (c.ProcessId !== p.ProcessId) stack.push(c);
  }
  return out;
}

const mb = (b) => Math.round(b / 1024 / 1024);
function snapshot(rootPid) {
  const t = tree(rootPid);
  return {
    processes: t.length,
    names: [...new Set(t.map((p) => p.Name))].join(", "),
    workingSetMB: mb(t.reduce((s, p) => s + Number(p.WorkingSetSize), 0)),
    privateMB: mb(t.reduce((s, p) => s + Number(p.PrivatePageCount), 0)),
    cpu100ns: t.reduce((s, p) => s + Number(p.KernelModeTime) + Number(p.UserModeTime), 0),
  };
}

async function cdp(expr) {
  const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("no page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => ((ws.onopen = r), (ws.onerror = j)));
  const v = await new Promise((res) => {
    ws.onmessage = (e) => {
      const m = JSON.parse(e.data);
      if (m.id === 1) res(m.result?.result?.value);
    };
    ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: expr, awaitPromise: true, returnByValue: true } }));
  });
  ws.close();
  return v;
}

async function launch() {
  const t0 = performance.now();
  const child = spawn(exe, [], {
    env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}` },
    stdio: "ignore",
  });
  for (let i = 0; i < 300; i++) {
    const n = await cdp("document.querySelectorAll('.nav-item').length").catch(() => 0);
    if (n >= 9) return { child, readyMs: Math.round(performance.now() - t0) };
    await sleep(50);
  }
  child.kill();
  throw new Error("UI did not become ready in 15 s");
}

async function close(child) {
  execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  await sleep(1500);
}

const cores = Number(execFileSync("powershell", ["-NoProfile", "-Command", "(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors"], { encoding: "utf8" }).trim());
const result = { exe, cores, startupMs: [], idle: null, connected: null };

for (let r = 0; r < runs; r++) {
  const { child, readyMs } = await launch();
  result.startupMs.push(readyMs);
  if (r < runs - 1) await close(child);
  else {
    // Idle: no sessions; the default view stays open.
    await sleep(Math.max(0, idle - 60) * 1000);
    const a = snapshot(child.pid);
    await sleep(60_000);
    const b = snapshot(child.pid);
    const cpu = (b.cpu100ns - a.cpu100ns) / 1e7 / 60; // core-seconds per second
    result.idle = { ...b, cpu100ns: undefined, cpuPctOfOneCore: +(cpu * 100).toFixed(2), cpuPctOfMachine: +((cpu * 100) / cores).toFixed(2) };
    if (connect) {
      await cdp(`(async () => {
        const { invoke } = await import('/node_modules/.vite/deps/@tauri-apps_api_core.js').catch(() => ({ invoke: window.__TAURI_INTERNALS__.invoke }));
        const h = (await invoke('hosts_list')).find((x) => !x.is_local);
        if (h) await invoke('host_connect', { id: h.id });
        [...document.querySelectorAll('.nav-item')].find((x) => x.textContent.startsWith('Devices'))?.click();
      })()`);
      await sleep(30_000);
      const c = snapshot(child.pid);
      await sleep(30_000);
      const d = snapshot(child.pid);
      const cpu2 = (d.cpu100ns - c.cpu100ns) / 1e7 / 30;
      result.connected = { ...d, cpu100ns: undefined, cpuPctOfOneCore: +(cpu2 * 100).toFixed(2), cpuPctOfMachine: +((cpu2 * 100) / cores).toFixed(2) };
    }
    await close(child);
  }
}
const sorted = [...result.startupMs].sort((x, y) => x - y);
result.startupP50Ms = sorted[Math.floor(sorted.length / 2)];
console.log(JSON.stringify(result, null, 2));
