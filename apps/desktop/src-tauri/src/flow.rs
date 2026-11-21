//! Developer workflow (FLOW-01..03): build a Flutter APK on a chosen build
//! host, install it on an Android device reached through *its own* route
//! (possibly behind another host), optionally launch it and show logcat.
//!
//! Every step reports status, a failure stops the run, the run can be
//! cancelled, and a summary of each run is kept as an audit trail. Build
//! commands are fixed argument lists; nothing user-typed reaches a shell.

use crate::android::{resolve, Android};
use crate::android_tools::{confirm_target, push_file, valid_package};
use crate::logging;
use crate::state::{db_err, to_core, AppState, CmdResult};
use senvakara_adapters_adb::AdbServer;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::RunRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

const STEPS: &[&str] = &[
    "validate", "build", "artifact", "install", "launch", "logcat",
];

#[derive(Default)]
pub struct Flows {
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlowSpec {
    /// Build host. Only this PC is supported as build host for now.
    pub build_host_id: String,
    pub project_dir: String,
    /// `debug` or `release`.
    pub mode: String,
    pub route_id: String,
    pub expected_serial: String,
    pub launch: bool,
    pub logcat: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowEvent {
    Step {
        ordinal: usize,
        name: String,
        status: String,
        detail: Option<String>,
    },
    Log {
        text: String,
    },
    Done {
        status: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct FlutterInfo {
    pub path: String,
    pub version: String,
}

/// Find the Flutter SDK on this PC: PATH, FLUTTER_ROOT, then common folders.
pub fn find_local_flutter() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "flutter.bat"
    } else {
        "flutter"
    };
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("PATH") {
        c.extend(std::env::split_paths(&p).map(|d| d.join(exe)));
    }
    if let Some(r) = std::env::var_os("FLUTTER_ROOT") {
        c.push(PathBuf::from(r).join("bin").join(exe));
    }
    for var in ["USERPROFILE", "LOCALAPPDATA", "HOME"] {
        if let Some(h) = std::env::var_os(var) {
            c.push(PathBuf::from(&h).join("flutter").join("bin").join(exe));
            c.push(
                PathBuf::from(&h)
                    .join("development")
                    .join("flutter")
                    .join("bin")
                    .join(exe),
            );
        }
    }
    c.push(PathBuf::from(r"C:\src\flutter\bin").join(exe));
    c.into_iter().find(|p| p.is_file())
}

/// `applicationId` from `android/app/build.gradle(.kts)` (Kotlin or Groovy DSL).
pub fn application_id(gradle: &str) -> Option<String> {
    gradle.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("applicationId")?;
        let rest = rest.trim_start().trim_start_matches('=').trim();
        let id = rest.trim_matches(|c| c == '"' || c == '\'').trim();
        valid_package(id).then(|| id.to_string())
    })
}

/// A command for the Flutter SDK. `flutter.bat` runs through `cmd.exe`; it
/// must not inherit this GUI process's console/stdin (that made `cmd.exe`
/// fail with 0xC0000142) and must not flash a console window.
fn flutter_command(path: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(path);
    c.stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

#[tauri::command]
pub async fn flutter_info(state: State<'_, AppState>, host_id: String) -> CmdResult<FlutterInfo> {
    let host = state
        .db
        .lock()
        .unwrap()
        .get_host(&host_id)
        .map_err(|e| db_err("flutter", e))?;
    if !host.is_local {
        return Err(CoreError::new(
            ErrorCode::UnsupportedCapability,
            "only this PC can be the build host for now (no Flutter SDK on the Linux reference host)",
        ));
    }
    let path = find_local_flutter().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnsupportedCapability,
            "Flutter SDK not found on this PC",
        )
    })?;
    let out = flutter_command(&path)
        .arg("--version")
        .output()
        .await
        .map_err(|e| to_core("flutter --version", e.into()))?;
    if !out.status.success() {
        return Err(CoreError::new(
            ErrorCode::InternalError,
            format!("flutter --version failed ({})", out.status),
        ));
    }
    let text = format!(
        "{}
{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let version = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("Flutter "))
        .unwrap_or("Flutter (version unknown)")
        .to_string();
    Ok(FlutterInfo {
        path: path.display().to_string(),
        version,
    })
}

#[tauri::command]
pub fn flow_history(state: State<'_, AppState>) -> CmdResult<Vec<RunRecord>> {
    state
        .db
        .lock()
        .unwrap()
        .list_runs(20)
        .map_err(|e| db_err("history", e))
}

#[tauri::command]
pub fn flow_cancel(flows: State<'_, Flows>, run_id: String) {
    if let Some(c) = flows.cancel.lock().unwrap().get(&run_id) {
        c.store(true, Ordering::Relaxed);
    }
}

struct Run {
    app: AppHandle,
    id: String,
    ch: Channel<FlowEvent>,
    cancel: Arc<AtomicBool>,
}

impl Run {
    fn step(&self, ordinal: usize, status: &str, detail: Option<String>) {
        let result = detail.as_ref().map(|d| serde_json::json!({ "detail": d }));
        let _ = self.app.state::<AppState>().db.lock().unwrap().set_step(
            &self.id,
            ordinal,
            status,
            result.as_ref(),
        );
        let _ = self.ch.send(FlowEvent::Step {
            ordinal,
            name: STEPS[ordinal].into(),
            status: status.into(),
            detail,
        });
    }
    fn log(&self, text: impl Into<String>) {
        let _ = self.ch.send(FlowEvent::Log { text: text.into() });
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

fn fail(code: ErrorCode, msg: impl Into<String>) -> CoreError {
    CoreError::new(code, msg)
}

#[tauri::command]
pub async fn flow_start(
    app: AppHandle,
    state: State<'_, AppState>,
    android: State<'_, Android>,
    flows: State<'_, Flows>,
    spec: FlowSpec,
    on_event: Channel<FlowEvent>,
) -> CmdResult<String> {
    if !["debug", "release"].contains(&spec.mode.as_str()) {
        return Err(fail(
            ErrorCode::InvalidInput,
            "mode must be debug or release",
        ));
    }
    // Resolve and confirm the target before anything starts.
    let (record, server, transport) = resolve(&state, &android, &spec.route_id).await?;
    confirm_target(&record, &spec.expected_serial)?;
    let summary = serde_json::json!({
        "build_host": spec.build_host_id, "project": spec.project_dir, "mode": spec.mode,
        "target": format!("{} ({} · {})", record.display_name, record.host_name, record.serial),
    });
    let steps: Vec<&str> = STEPS.to_vec();
    let id = state
        .db
        .lock()
        .unwrap()
        .create_run(&summary, &steps)
        .map_err(|e| db_err("run", e))?;
    let cancel = Arc::new(AtomicBool::new(false));
    flows
        .cancel
        .lock()
        .unwrap()
        .insert(id.clone(), cancel.clone());
    logging::info(&format!(
        "flow {id} started → route {} (host {})",
        spec.route_id, record.host_id
    ));

    let run = Run {
        app: app.clone(),
        id: id.clone(),
        ch: on_event,
        cancel,
    };
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let outcome = execute(&run, &spec, server, transport, &record.display_name).await;
        let (status, message) = match &outcome {
            Ok(msg) => ("succeeded", msg.clone()),
            Err(e) if e.code == ErrorCode::Cancelled => ("cancelled", e.message.clone()),
            Err(e) => ("failed", format!("{:?}: {}", e.code, e.message)),
        };
        let mut final_summary = summary.clone();
        final_summary["result"] = serde_json::json!(message);
        final_summary["seconds"] = serde_json::json!(started.elapsed().as_secs());
        let _ = run.app.state::<AppState>().db.lock().unwrap().finish_run(
            &run.id,
            status,
            &final_summary,
        );
        let _ = run.ch.send(FlowEvent::Done {
            status: status.into(),
            message: message.clone(),
        });
        run.app
            .state::<Flows>()
            .cancel
            .lock()
            .unwrap()
            .remove(&run.id);
        logging::info(&format!("flow {} finished: {status}", run.id));
    });
    Ok(id)
}

async fn execute(
    run: &Run,
    spec: &FlowSpec,
    server: AdbServer,
    t: u64,
    device_name: &str,
) -> Result<String, CoreError> {
    let check_cancel = |step: usize| -> Result<(), CoreError> {
        if run.cancelled() {
            run.step(step, "cancelled", None);
            return Err(fail(ErrorCode::Cancelled, "cancelled by user"));
        }
        Ok(())
    };

    // 0. validate
    run.step(0, "running", None);
    let host_is_local = run
        .app
        .state::<AppState>()
        .db
        .lock()
        .unwrap()
        .get_host(&spec.build_host_id)
        .map(|h| h.is_local)
        .unwrap_or(false);
    if !host_is_local {
        let e = fail(
            ErrorCode::UnsupportedCapability,
            "only this PC can be the build host for now",
        );
        run.step(0, "failed", Some(e.message.clone()));
        return Err(e);
    }
    let flutter = match find_local_flutter() {
        Some(f) => f,
        None => {
            let e = fail(
                ErrorCode::UnsupportedCapability,
                "Flutter SDK not found on this PC",
            );
            run.step(0, "failed", Some(e.message.clone()));
            return Err(e);
        }
    };
    let project = PathBuf::from(&spec.project_dir);
    let gradle = ["build.gradle.kts", "build.gradle"]
        .iter()
        .map(|f| project.join("android").join("app").join(f))
        .find(|p| p.is_file());
    let package = gradle
        .as_ref()
        .and_then(|g| std::fs::read_to_string(g).ok())
        .and_then(|g| application_id(&g));
    if !project.join("pubspec.yaml").is_file() || gradle.is_none() {
        let e = fail(
            ErrorCode::InvalidInput,
            "not a Flutter project with an Android app (pubspec.yaml / android/app)",
        );
        run.step(0, "failed", Some(e.message.clone()));
        return Err(e);
    }
    run.step(
        0,
        "succeeded",
        Some(format!(
            "Flutter at {} · package {}",
            flutter.display(),
            package.as_deref().unwrap_or("?")
        )),
    );
    check_cancel(1)?;

    // 1. build
    run.step(
        1,
        "running",
        Some(format!("flutter build apk --{}", spec.mode)),
    );
    let mut child = flutter_command(&flutter)
        .args(["build", "apk", &format!("--{}", spec.mode)])
        .current_dir(&project)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            run.step(1, "failed", Some(e.to_string()));
            to_core("flutter build", e.into())
        })?;
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut err = BufReader::new(child.stderr.take().unwrap()).lines();
    let build_started = Instant::now();
    let status = loop {
        if run.cancelled() {
            // flutter.bat → cmd.exe → dart: stop the whole tree, not just cmd.
            #[cfg(windows)]
            if let Some(pid) = child.id() {
                let mut tk = tokio::process::Command::new("taskkill");
                tk.args(["/T", "/F", "/PID", &pid.to_string()])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(0x0800_0000);
                let _ = tk.status().await;
            }
            let _ = child.kill().await;
            run.step(1, "cancelled", None);
            return Err(fail(ErrorCode::Cancelled, "build cancelled"));
        }
        tokio::select! {
            Ok(Some(l)) = out.next_line() => run.log(l),
            Ok(Some(l)) = err.next_line() => run.log(l),
            s = child.wait() => break s,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    };
    // Drain whatever is left after exit.
    while let Ok(Some(l)) = out.next_line().await {
        run.log(l);
    }
    match status {
        Ok(s) if s.success() => run.step(
            1,
            "succeeded",
            Some(format!("{} s", build_started.elapsed().as_secs())),
        ),
        Ok(s) => {
            let e = fail(
                ErrorCode::InternalError,
                format!("flutter build exited with {s}"),
            );
            run.step(1, "failed", Some(e.message.clone()));
            return Err(e);
        }
        Err(e) => {
            run.step(1, "failed", Some(e.to_string()));
            return Err(to_core("flutter build", e.into()));
        }
    }
    check_cancel(2)?;

    // 2. artifact
    run.step(2, "running", None);
    let apk = project
        .join("build")
        .join("app")
        .join("outputs")
        .join("flutter-apk")
        .join(format!("app-{}.apk", spec.mode));
    let bytes = match tokio::fs::read(&apk).await {
        Ok(b) => b,
        Err(e) => {
            run.step(
                2,
                "failed",
                Some(format!("{} not found: {e}", apk.display())),
            );
            return Err(fail(ErrorCode::InternalError, "APK not found after build"));
        }
    };
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let size = bytes.len();
    drop(bytes);
    run.step(
        2,
        "succeeded",
        Some(format!(
            "{} · {} bytes · sha256 {}",
            file_label(&apk),
            size,
            &sha[..16]
        )),
    );
    check_cancel(3)?;

    // 3. install on the target route (device may be behind another host)
    run.step(3, "running", Some(format!("to {device_name}")));
    let remote = format!("/data/local/tmp/senvakara-flow-{}.apk", &sha[..12]);
    if let Err(e) = push_file(server, t, &apk, &remote, &run.cancel, |_| {}).await {
        let ce = to_core("push apk", e);
        run.step(
            3,
            if ce.code == ErrorCode::Cancelled {
                "cancelled"
            } else {
                "failed"
            },
            Some(ce.message.clone()),
        );
        return Err(ce);
    }
    let installed = server
        .shell(
            t,
            &format!("pm install -r {remote}"),
            Duration::from_secs(180),
            64 * 1024,
        )
        .await
        .map_err(|e| to_core("pm install", e));
    let _ = server
        .shell(t, &format!("rm -f {remote}"), Duration::from_secs(10), 1024)
        .await;
    let text = installed?.output.trim().to_string();
    if !text.contains("Success") {
        run.step(3, "failed", Some(text.clone()));
        return Err(fail(ErrorCode::Conflict, format!("install failed: {text}")));
    }
    run.step(3, "succeeded", Some(format!("installed on {device_name}")));

    // 4. launch (optional)
    let mut pid = None;
    if spec.launch {
        check_cancel(4)?;
        run.step(4, "running", None);
        let Some(pkg) = package.clone() else {
            run.step(
                4,
                "failed",
                Some("applicationId not found in build.gradle".into()),
            );
            return Err(fail(
                ErrorCode::InvalidInput,
                "cannot launch: unknown package",
            ));
        };
        let _ = server
            .shell(
                t,
                &format!("monkey -p {pkg} -c android.intent.category.LAUNCHER 1"),
                Duration::from_secs(20),
                16 * 1024,
            )
            .await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Ok(o) = server
                .shell(t, &format!("pidof {pkg}"), Duration::from_secs(5), 256)
                .await
            {
                let p = o
                    .output
                    .split_whitespace()
                    .next()
                    .map(str::to_string);
                if p.as_deref()
                    .is_some_and(|p| p.chars().all(|c| c.is_ascii_digit()))
                {
                    pid = p;
                    break;
                }
            }
        }
        match &pid {
            Some(p) => run.step(4, "succeeded", Some(format!("{pkg} running (pid {p})"))),
            None => {
                run.step(4, "failed", Some(format!("{pkg} did not start")));
                return Err(fail(ErrorCode::InternalError, "app did not start"));
            }
        }
    }

    // 5. logcat of the launched app for a short window (optional)
    if spec.logcat {
        if let Some(p) = &pid {
            check_cancel(5)?;
            run.step(5, "running", Some("15 s".into()));
            let mut s = server
                .open_service(t, &format!("shell:logcat -v time --pid={p}"))
                .await
                .map_err(|e| to_core("logcat", e))?;
            let until = Instant::now() + Duration::from_secs(15);
            let mut buf = vec![0u8; 16 * 1024];
            let mut lines = 0usize;
            while Instant::now() < until && !run.cancelled() {
                match tokio::time::timeout(Duration::from_millis(500), s.read(&mut buf)).await {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Ok(Ok(n)) => {
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        lines += text.lines().count();
                        for l in text.lines() {
                            run.log(format!("[logcat] {l}"));
                        }
                    }
                    Err(_) => {}
                }
            }
            run.step(5, "succeeded", Some(format!("{lines} lines")));
        }
    }
    Ok(format!(
        "deployed to {device_name}{}",
        if pid.is_some() { " and running" } else { "" }
    ))
}

fn file_label(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::application_id;

    #[test]
    fn reads_application_id_from_both_dsls() {
        let kts = "android {\n    defaultConfig {\n        applicationId = \"dev.senvakara.senvakara_hello\"\n    }\n}";
        assert_eq!(
            application_id(kts).as_deref(),
            Some("dev.senvakara.senvakara_hello")
        );
        let groovy = "defaultConfig {\n  applicationId 'com.example.app'\n}";
        assert_eq!(application_id(groovy).as_deref(), Some("com.example.app"));
        assert_eq!(application_id("applicationId = \"x;rm -rf /\""), None);
        assert_eq!(application_id("namespace = \"a.b\""), None);
    }
}
