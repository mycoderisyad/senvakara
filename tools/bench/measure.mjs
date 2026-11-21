// Whole-process-tree benchmark for the Windows app (task QA-03).
//
// Launches the given exe, measures the time until its main window exists, then samples memory
// and CPU of the app process plus every descendant (shells it started, for example).
//
// Usage: node tools/bench/measure.mjs <path-to-exe> [--idle 300] [--runs 3]
//   --idle  seconds to stay idle on Home before the 60 s CPU measurement ends (spec: 300)
//   --runs  startup runs (each launch is closed before the next)
//
// Only one Senvakara can run per data folder: close the app first, or set SENVAKARA_DATA_DIR to
// a scratch folder to measure an empty inventory.
import { spawn, execFileSync } from "node:child_process";

const args = process.argv.slice(2);
const exe = args[0];
if (!exe) {
  console.error("usage: node tools/bench/measure.mjs <path-to-exe> [--idle 300] [--runs 3]");
  process.exit(2);
}
const opt = (name, def) => {
  const i = args.indexOf(name);
  return i < 0 ? def : Number(args[i + 1]);
};
const idle = opt("--idle", 300);
const runs = opt("--runs", 3);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ps = (script) => execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).trim();

function procs() {
  return JSON.parse(
    ps("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,WorkingSetSize,PrivatePageCount,KernelModeTime,UserModeTime | ConvertTo-Json -Compress"),
  );
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

// Ready = the process owns a main window. The wait runs inside one PowerShell call so the
// 20 ms polling does not pay a process start each time; its own start-up (about 0.2 s) is
// measured once and subtracted.
const psStartMs = (() => {
  const t0 = performance.now();
  ps("1");
  return performance.now() - t0;
})();

async function launch() {
  const t0 = performance.now();
  const child = spawn(exe, [], { stdio: "ignore" });
  const waited = Number(
    ps(`$p = Get-Process -Id ${child.pid}; $sw = [Diagnostics.Stopwatch]::StartNew(); while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 15000) { Start-Sleep -Milliseconds 20; $p.Refresh() }; if ($p.MainWindowHandle -eq 0) { -1 } else { $sw.ElapsedMilliseconds }`),
  );
  if (waited < 0) {
    child.kill();
    throw new Error("the window did not appear in 15 s (is another Senvakara running?)");
  }
  return { child, readyMs: Math.max(0, Math.round(performance.now() - t0 - psStartMs)) };
}

async function close(child) {
  execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  await sleep(1500);
}

const cores = Number(ps("(Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors"));
const result = { exe, cores, startupMs: [], idle: null };

for (let r = 0; r < runs; r++) {
  const { child, readyMs } = await launch();
  result.startupMs.push(readyMs);
  if (r < runs - 1) await close(child);
  else {
    // Idle: no sessions; Home stays open.
    await sleep(Math.max(0, idle - 60) * 1000);
    const a = snapshot(child.pid);
    await sleep(60_000);
    const b = snapshot(child.pid);
    const cpu = (b.cpu100ns - a.cpu100ns) / 1e7 / 60; // core-seconds per second
    result.idle = { ...b, cpu100ns: undefined, cpuPctOfOneCore: +(cpu * 100).toFixed(2), cpuPctOfMachine: +((cpu * 100) / cores).toFixed(2) };
    await close(child);
  }
}
const sorted = [...result.startupMs].sort((x, y) => x - y);
result.startupP50Ms = sorted[Math.floor(sorted.length / 2)];
console.log(JSON.stringify(result, null, 2));
