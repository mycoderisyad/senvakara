//! Automation recipes (FLOW-04, task FLOW-IMPL-04): reusable, allowlisted
//! step lists bound to one target device, run with explicit permissions,
//! cancellable, stopped on the first failure and recorded in run history.

use crate::android::resolve;
use crate::android_tools::{confirm_target, push_file, valid_package};
use crate::flow::{run_deploy, FlowEvent, FlowSpec};
use crate::logging;
use crate::sink::Sink;
use crate::state::{db_err, to_core, CmdResult};
use crate::App;
use senvakara_adapters_adb::AdbServer;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::{RecipePermissions, RecipeRecord, RecipeSave, RecipeStep};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidInput, msg)
}

fn str_param<'a>(s: &'a RecipeStep, key: &str) -> CmdResult<&'a str> {
    s.params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad(format!("step {}: missing `{key}`", s.kind)))
}

/// Check every step's parameters and permissions before anything runs.
pub fn check_plan(steps: &[RecipeStep], perms: &RecipePermissions) -> CmdResult<()> {
    for (i, s) in steps.iter().enumerate() {
        let n = i + 1;
        match s.kind.as_str() {
            "install_apk" | "uninstall_package" | "flutter_deploy" if !perms.install => {
                return Err(CoreError::new(
                    ErrorCode::Unauthorized,
                    format!("step {n} ({}) needs the \"install / uninstall\" permission for this recipe", s.kind),
                ))
            }
            "device_shell" if !perms.shell => {
                return Err(CoreError::new(
                    ErrorCode::Unauthorized,
                    format!("step {n} (device_shell) needs the \"run shell commands\" permission for this recipe"),
                ))
            }
            _ => {}
        }
        match s.kind.as_str() {
            "install_apk" => {
                let p = str_param(s, "path")?;
                if !p.to_ascii_lowercase().ends_with(".apk") {
                    return Err(bad(format!("step {n}: path must point to an .apk")));
                }
            }
            "uninstall_package" | "launch_app" => {
                if !valid_package(str_param(s, "package")?) {
                    return Err(bad(format!("step {n}: invalid package name")));
                }
            }
            "wait" => {
                let secs = s
                    .params
                    .get("seconds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if !(1..=300).contains(&secs) {
                    return Err(bad(format!("step {n}: wait must be 1-300 seconds")));
                }
            }
            "device_shell" => {
                if str_param(s, "command")?.len() > 4096 {
                    return Err(bad(format!("step {n}: command too long")));
                }
            }
            "screenshot" => {
                str_param(s, "dir")?;
            }
            "flutter_deploy" => {
                str_param(s, "project_dir")?;
                let mode = s
                    .params
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("release");
                if !["debug", "release"].contains(&mode) {
                    return Err(bad(format!("step {n}: mode must be debug or release")));
                }
            }
            other => return Err(bad(format!("step {n}: unknown step kind {other}"))),
        }
    }
    Ok(())
}

pub(crate) fn recipes_list(app: &App) -> CmdResult<Vec<RecipeRecord>> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .list_recipes()
        .map_err(|e| db_err("recipes", e))
}

pub(crate) fn recipe_save(app: &App, input: RecipeSave) -> CmdResult<RecipeRecord> {
    let state = &app.state;
    // Validate parameters now so a broken recipe is never stored; permissions
    // are checked again at run time.
    check_plan(
        &input.steps,
        &RecipePermissions {
            install: true,
            shell: true,
        },
    )?;
    state
        .db
        .lock()
        .unwrap()
        .save_recipe(&input)
        .map_err(|e| db_err("save recipe", e))
}

pub(crate) fn recipe_delete(app: &App, id: String) -> CmdResult<()> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .delete_recipe(&id)
        .map_err(|e| db_err("delete recipe", e))
}

struct Ctx {
    app: Arc<App>,
    run_id: String,
    sink: Sink<FlowEvent>,
    cancel: Arc<AtomicBool>,
    server: AdbServer,
    t: u64,
    route_id: String,
    serial: String,
    device_name: String,
}

impl Ctx {
    fn step(&self, ordinal: usize, name: &str, status: &str, detail: Option<String>) {
        let result = detail.as_ref().map(|d| serde_json::json!({ "detail": d }));
        let _ = self.app.state.db.lock().unwrap().set_step(
            &self.run_id,
            ordinal,
            status,
            result.as_ref(),
        );
        (self.sink)(FlowEvent::Step {
            ordinal,
            name: name.into(),
            status: status.into(),
            detail,
        });
    }
    fn log(&self, text: impl Into<String>) {
        (self.sink)(FlowEvent::Log { text: text.into() });
    }
    async fn shell(&self, cmd: &str, secs: u64) -> CmdResult<String> {
        self.server
            .shell(self.t, cmd, Duration::from_secs(secs), 64 * 1024)
            .await
            .map(|o| o.output)
            .map_err(|e| to_core("recipe shell", e))
    }
}

/// Run a saved recipe against its target device.
pub(crate) async fn recipe_run(
    app: &Arc<App>,
    recipe_id: String,
    expected_serial: String,
    sink: Sink<FlowEvent>,
) -> CmdResult<String> {
    let state = &app.state;
    let android = &app.android;
    let flows = &app.flows;
    let recipe = state
        .db
        .lock()
        .unwrap()
        .get_recipe(&recipe_id)
        .map_err(|e| db_err("recipe", e))?;
    check_plan(&recipe.steps, &recipe.permissions)?;
    let route_id = recipe
        .target_route_id
        .clone()
        .ok_or_else(|| bad("this recipe has no target device"))?;
    let (record, server, t) = resolve(state, android, &route_id).await?;
    confirm_target(&record, &expected_serial)?;

    let names: Vec<String> = recipe.steps.iter().map(|s| s.kind.clone()).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let summary = serde_json::json!({
        "recipe": recipe.name,
        "target": format!("{} ({} · {})", record.display_name, record.host_name, record.serial),
    });
    let run_id = {
        let mut db = state.db.lock().unwrap();
        let id = db
            .create_run(&summary, &name_refs)
            .map_err(|e| db_err("run", e))?;
        db.set_run_recipe(&id, &recipe.id)
            .map_err(|e| db_err("run", e))?;
        id
    };
    let cancel = Arc::new(AtomicBool::new(false));
    flows.insert(&run_id, cancel.clone());
    logging::info(&format!(
        "recipe {} run {run_id} started → route {route_id}",
        recipe.id
    ));

    let ctx = Ctx {
        app: app.clone(),
        run_id: run_id.clone(),
        sink,
        cancel,
        server,
        t,
        route_id,
        serial: record.serial.clone(),
        device_name: record.display_name.clone(),
    };
    app.rt.spawn(async move {
        let started = Instant::now();
        let outcome = execute(&ctx, &recipe).await;
        let (status, message) = match &outcome {
            Ok(m) => ("succeeded", m.clone()),
            Err(e) if e.code == ErrorCode::Cancelled => ("cancelled", e.message.clone()),
            Err(e) => ("failed", format!("{:?}: {}", e.code, e.message)),
        };
        let mut s = summary;
        s["result"] = serde_json::json!(message);
        s["seconds"] = serde_json::json!(started.elapsed().as_secs());
        let _ = ctx
            .app
            .state
            .db
            .lock()
            .unwrap()
            .finish_run(&ctx.run_id, status, &s);
        (ctx.sink)(FlowEvent::Done {
            status: status.into(),
            message,
        });
        ctx.app.flows.remove(&ctx.run_id);
        logging::info(&format!("recipe run {} finished: {status}", ctx.run_id));
    });
    Ok(run_id)
}

async fn execute(ctx: &Ctx, recipe: &RecipeRecord) -> Result<String, CoreError> {
    for (i, s) in recipe.steps.iter().enumerate() {
        if ctx.cancel.load(Ordering::Relaxed) {
            ctx.step(i, &s.kind, "cancelled", None);
            return Err(CoreError::new(ErrorCode::Cancelled, "cancelled by user"));
        }
        ctx.step(i, &s.kind, "running", None);
        match run_step(ctx, s).await {
            Ok(detail) => ctx.step(i, &s.kind, "succeeded", Some(detail)),
            Err(e) => {
                let status = if e.code == ErrorCode::Cancelled {
                    "cancelled"
                } else {
                    "failed"
                };
                ctx.step(i, &s.kind, status, Some(e.message.clone()));
                return Err(e);
            }
        }
    }
    Ok(format!(
        "{} steps on {}",
        recipe.steps.len(),
        ctx.device_name
    ))
}

async fn run_step(ctx: &Ctx, s: &RecipeStep) -> Result<String, CoreError> {
    match s.kind.as_str() {
        "install_apk" => {
            let path = PathBuf::from(str_param(s, "path")?);
            let remote = format!(
                "/data/local/tmp/senvakara-recipe-{}.apk",
                uuid::Uuid::new_v4().simple()
            );
            push_file(ctx.server, ctx.t, &path, &remote, &ctx.cancel, |_| {})
                .await
                .map_err(|e| to_core("push apk", e))?;
            let out = ctx.shell(&format!("pm install -r {remote}"), 180).await;
            let _ = ctx.shell(&format!("rm -f {remote}"), 10).await;
            let out = out?;
            if out.contains("Success") {
                Ok("installed".into())
            } else {
                Err(CoreError::new(
                    ErrorCode::Conflict,
                    format!("install failed: {}", out.trim()),
                ))
            }
        }
        "uninstall_package" => {
            let pkg = str_param(s, "package")?;
            let out = ctx.shell(&format!("pm uninstall {pkg}"), 60).await?;
            if out.contains("Success") {
                Ok(format!("{pkg} removed"))
            } else {
                Err(CoreError::new(
                    ErrorCode::Conflict,
                    format!("uninstall failed: {}", out.trim()),
                ))
            }
        }
        "launch_app" => {
            let pkg = str_param(s, "package")?;
            ctx.shell(
                &format!("monkey -p {pkg} -c android.intent.category.LAUNCHER 1"),
                20,
            )
            .await?;
            for _ in 0..20 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let out = ctx
                    .shell(&format!("pidof {pkg}"), 5)
                    .await
                    .unwrap_or_default();
                if let Some(pid) = out
                    .split_whitespace()
                    .next()
                    .filter(|p| p.chars().all(|c| c.is_ascii_digit()))
                {
                    return Ok(format!("{pkg} running (pid {pid})"));
                }
            }
            Err(CoreError::new(
                ErrorCode::InternalError,
                format!("{pkg} did not start"),
            ))
        }
        "wait" => {
            let secs = s
                .params
                .get("seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until {
                if ctx.cancel.load(Ordering::Relaxed) {
                    return Err(CoreError::new(
                        ErrorCode::Cancelled,
                        "cancelled during wait",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok(format!("{secs} s"))
        }
        "device_shell" => {
            let cmd = str_param(s, "command")?;
            logging::info(&format!(
                "recipe shell on route {} (transport {})",
                ctx.route_id, ctx.t
            ));
            let out = ctx.shell(cmd, 60).await?;
            for l in out.lines().take(200) {
                ctx.log(format!("[shell] {l}"));
            }
            Ok(format!("{} lines of output", out.lines().count()))
        }
        "screenshot" => {
            let dir = PathBuf::from(str_param(s, "dir")?);
            let png = ctx
                .server
                .exec_bytes(
                    ctx.t,
                    "screencap -p",
                    Duration::from_secs(20),
                    48 * 1024 * 1024,
                )
                .await
                .map_err(|e| to_core("screenshot", e))?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let file = dir.join(format!("senvakara-{}-{stamp}.png", ctx.serial));
            tokio::fs::write(&file, png)
                .await
                .map_err(|e| to_core("save screenshot", e.into()))?;
            Ok(file.display().to_string())
        }
        "flutter_deploy" => {
            let local = ctx
                .app
                .state
                .db
                .lock()
                .unwrap()
                .list_hosts()
                .ok()
                .and_then(|h| h.into_iter().find(|x| x.is_local))
                .ok_or_else(|| CoreError::internal("local host missing"))?;
            let spec = FlowSpec {
                build_host_id: local.id,
                project_dir: str_param(s, "project_dir")?.to_string(),
                mode: s
                    .params
                    .get("mode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("release")
                    .to_string(),
                route_id: ctx.route_id.clone(),
                expected_serial: ctx.serial.clone(),
                launch: s
                    .params
                    .get("launch")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                logcat: false,
            };
            let outer = ctx.sink.clone();
            let nested: Sink<FlowEvent> = Arc::new(move |e| match e {
                FlowEvent::Step { name, status, .. } => outer(FlowEvent::Log {
                    text: format!("[deploy] {name}: {status}"),
                }),
                FlowEvent::Log { text } => outer(FlowEvent::Log { text }),
                FlowEvent::Done { .. } => true,
            });
            run_deploy(&ctx.app, spec, ctx.cancel.clone(), nested).await
        }
        other => Err(bad(format!("unknown step kind {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(kind: &str, params: serde_json::Value) -> RecipeStep {
        RecipeStep {
            kind: kind.into(),
            params,
        }
    }

    #[test]
    fn permissions_are_required() {
        let none = RecipePermissions::default();
        let shell = vec![step("device_shell", json!({"command": "id"}))];
        assert_eq!(
            check_plan(&shell, &none).unwrap_err().code,
            ErrorCode::Unauthorized
        );
        assert!(check_plan(
            &shell,
            &RecipePermissions {
                shell: true,
                install: false
            }
        )
        .is_ok());
        let install = vec![step("uninstall_package", json!({"package": "dev.x.app"}))];
        assert_eq!(
            check_plan(&install, &none).unwrap_err().code,
            ErrorCode::Unauthorized
        );
        let harmless = vec![
            step("wait", json!({"seconds": 2})),
            step("launch_app", json!({"package": "dev.x.app"})),
        ];
        assert!(check_plan(&harmless, &none).is_ok());
    }

    #[test]
    fn parameters_are_validated() {
        let all = RecipePermissions {
            install: true,
            shell: true,
        };
        for bad_step in [
            step("wait", json!({"seconds": 0})),
            step("wait", json!({"seconds": 999})),
            step("launch_app", json!({"package": "x; reboot"})),
            step("install_apk", json!({"path": "C:\\a.exe"})),
            step(
                "flutter_deploy",
                json!({"project_dir": "D:\\p", "mode": "profile"}),
            ),
            step("teleport", json!({})),
        ] {
            assert!(
                check_plan(std::slice::from_ref(&bad_step), &all).is_err(),
                "{bad_step:?}"
            );
        }
    }
}
