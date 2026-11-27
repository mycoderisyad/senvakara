//! End-to-end check of the workflow run engine (WF-IMPL-01, ADR-030) against
//! the throw-away test host of WF-SPIKE-02. Registers the host from
//! `tools/spikes/ssh-exec/creds.local` in a throw-away data folder, runs
//! workflows through `App::workflow_run` and checks the outcome of every step,
//! then deletes the host again (with its stored password).
//!
//! Run (the owner sets up and tears down the host):
//!   wsl -d Ubuntu-24.04 -u root -- bash tools/spikes/ssh-exec/setup-wsl.sh
//!   cargo run -p senvakara-app --example wf_probe
//!   wsl -d Ubuntu-24.04 -u root -- bash tools/spikes/ssh-exec/teardown-wsl.sh

use anyhow::{bail, ensure, Context, Result};
use senvakara_app::workflow::{Binding, Edge, EdgeKind, Node, Permissions, Workflow};
use senvakara_app::{
    App, AppPaths, FlowEvent, HostUpsert, Protocol, Secret, Sink, SshAuth, WorkflowRunRequest,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

struct Creds {
    host: String,
    port: u16,
    user: String,
    password: String,
}

fn creds() -> Result<Creds> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/spikes/ssh-exec/creds.local"
    );
    let text =
        std::fs::read_to_string(path).context("read creds.local (run setup-wsl.sh first)")?;
    let get = |k: &str| -> Result<String> {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(|v| v.trim().to_string())
            .with_context(|| format!("creds.local: missing {k}"))
    };
    Ok(Creds {
        host: get("host")?,
        port: get("port")?.parse()?,
        user: get("user")?,
        password: get("password")?,
    })
}

fn node(id: &str, module: &str, x: f32) -> Node {
    Node {
        id: id.into(),
        module: module.into(),
        title: String::new(),
        x,
        y: 0.0,
        params: BTreeMap::new(),
        binding: None,
        scripts: BTreeMap::new(),
    }
}

fn script(id: &str, title: &str, check: Option<&str>, apply: &str, root: bool) -> Node {
    let mut n = node(id, "automation.custom_script", 300.0);
    n.title = title.into();
    n.scripts.insert("apply.sh".into(), apply.into());
    if let Some(c) = check {
        n.scripts.insert("check.sh".into(), c.into());
    }
    if root {
        n.params.insert("as_root".into(), serde_json::json!(true));
    }
    n
}

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        kind,
    }
}

/// A workflow with `actions` all running on `host_id`, chained by `after`.
fn graph(host_id: &str, actions: Vec<Node>, chain: &[(&str, &str)]) -> Workflow {
    let mut host = node("host", "host.ssh", 0.0);
    host.binding = Some(Binding::Host {
        host_id: host_id.into(),
    });
    let mut wf = Workflow {
        permissions: Permissions {
            root: true,
            shell: true,
            ..Permissions::default()
        },
        ..Workflow::default()
    };
    for a in &actions {
        wf.edges.push(edge(&a.id, "host", EdgeKind::Target));
    }
    for (a, b) in chain {
        wf.edges.push(edge(a, b, EdgeKind::After));
    }
    wf.nodes.push(host);
    wf.nodes.extend(actions);
    wf
}

/// Step title → (last status, its detail).
type Steps = HashMap<String, (String, String)>;

/// Runs a saved workflow and returns its final status and its steps.
fn run(
    ui: &tokio::runtime::Runtime,
    app: &Arc<App>,
    id: &str,
    sudo: Option<&str>,
    cancel_after: Option<Duration>,
) -> Result<(String, Steps)> {
    let (tx, rx) = mpsc::channel();
    let sink: Sink<FlowEvent> = Arc::new(move |e| tx.send(e).is_ok());
    let run_id = ui.block_on(app.workflow_run(
        WorkflowRunRequest {
            workflow_id: id.into(),
            sudo: sudo.map(|s| Secret::new(s.to_string())),
            serials: HashMap::new(),
        },
        sink,
    ))?;
    let started = Instant::now();
    let mut steps = HashMap::new();
    let mut cancelled = false;
    loop {
        if let Some(after) = cancel_after {
            if !cancelled && started.elapsed() >= after {
                app.flow_cancel(&run_id);
                cancelled = true;
            }
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(FlowEvent::Step {
                name,
                status,
                detail,
                ..
            }) => {
                println!(
                    "  step {name}: {status} {}",
                    detail.clone().unwrap_or_default()
                );
                steps.insert(name, (status, detail.unwrap_or_default()));
            }
            Ok(FlowEvent::Log { text }) => println!("    {text}"),
            Ok(FlowEvent::Done { status, message }) => {
                println!(
                    "  done: {status} ({message}) in {:.1}s",
                    started.elapsed().as_secs_f64()
                );
                return Ok((status, steps));
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() > Duration::from_secs(120) =>
            {
                bail!("run did not finish in 120 s")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => bail!("events ended: {e}"),
        }
    }
}

fn status(steps: &Steps, name: &str) -> String {
    steps.get(name).map(|s| s.0.clone()).unwrap_or_default()
}

fn main() -> Result<()> {
    let c = creds()?;
    let dir = std::env::temp_dir().join(format!("senvakara-wfprobe-{}", std::process::id()));
    let paths = AppPaths::in_dir(&dir);
    let backend = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = App::start(paths, backend.handle().clone())?;
    let ui = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let host = app.host_create(HostUpsert {
        display_name: "WF probe".into(),
        os_family: "linux".into(),
        protocol: Protocol::Ssh,
        address: c.host.clone(),
        port: c.port,
        username: c.user.clone(),
        auth: SshAuth::Password,
        group_name: None,
        tags: Vec::new(),
        startup_snippet: None,
    })?;
    let id = host.id.clone();
    let result = (|| -> Result<()> {
        ui.block_on(app.host_set_password(id.clone(), Some(c.password.clone())))?;
        let key = ui.block_on(app.host_check_key(id.clone()))?;
        // Loopback test host made by setup-wsl.sh; a real host is trusted by the user.
        ui.block_on(app.host_trust_key(id.clone(), key.presented))?;

        let marker = "$HOME/.cache/sv-wfprobe-marker";
        let check = format!("[ -f \"{marker}\" ] && exit 0 || exit 10\n");
        let apply = format!("touch \"{marker}\"\necho created as $(id -un)\necho a warning >&2\n");
        let wf = graph(
            &id,
            vec![
                script("mark", "Make marker", Some(&check), &apply, false),
                script(
                    "root",
                    "Root check",
                    None,
                    "test \"$(id -u)\" = 0\necho root ok\n",
                    true,
                ),
                script("fail", "Fails", None, "echo going to fail\nexit 3\n", false),
                script("after", "After failure", None, "echo must not run\n", false),
            ],
            &[("mark", "root"), ("fail", "after")],
        );
        let saved = app.workflow_save(None, "Probe".into(), &wf)?;

        // A marker left by an interrupted probe would make run 1 "up to date".
        // Internal calls run on the backend runtime, as `App`'s own methods do:
        // an SSH session started on the `ui` runtime stalls once `block_on` returns.
        let conn = backend.block_on(app.state.connection(&id))?;
        backend.block_on(conn.exec(&["sh", "-c", &format!("rm -f \"{marker}\"")]))?;

        println!("== 1. first run: apply, root via sudo, a failure skips what waits for it");
        let (s, steps) = run(&ui, &app, &saved.id, Some(&c.password), None)?;
        ensure!(s == "failed", "run status {s}");
        ensure!(status(&steps, "Make marker") == "succeeded");
        ensure!(steps["Make marker"].1 == "applied");
        ensure!(status(&steps, "Root check") == "succeeded");
        ensure!(status(&steps, "Fails") == "failed");
        ensure!(steps["Fails"].1.contains("exit status 3"));
        ensure!(status(&steps, "After failure") == "skipped");

        println!("== 2. second run: the check says the marker is there");
        let (_, steps) = run(&ui, &app, &saved.id, Some(&c.password), None)?;
        ensure!(
            steps["Make marker"].1 == "already up to date",
            "{:?}",
            steps["Make marker"]
        );

        println!("== 3. wrong sudo password: root step fails, others still run");
        let (_, steps) = run(&ui, &app, &saved.id, Some("definitely-wrong"), None)?;
        ensure!(status(&steps, "Root check") == "failed");
        ensure!(steps["Root check"]
            .1
            .contains("did not accept the sudo password"));
        ensure!(status(&steps, "Make marker") == "succeeded");

        println!("== 4. cancel ends a long script and everything it started");
        let long = graph(
            &id,
            vec![script(
                "long",
                "Long",
                None,
                "sleep 600 &\n( trap '' TERM; while :; do sleep 1; done ) &\nfor i in $(seq 1 600); do echo tick $i; sleep 1; done\n",
                true,
            )],
            &[],
        );
        let saved_long = app.workflow_save(None, "Long".into(), &long)?;
        let (s, steps) = run(
            &ui,
            &app,
            &saved_long.id,
            Some(&c.password),
            Some(Duration::from_secs(4)),
        )?;
        ensure!(s == "cancelled", "run status {s}");
        ensure!(status(&steps, "Long") == "cancelled");
        let conn = backend.block_on(app.state.connection(&id))?;
        let left = backend.block_on(conn.exec(&[
            "sh",
            "-c",
            "pgrep -fa 'slee[p] 600|whil[e] :' || true",
        ]))?;
        let left = String::from_utf8_lossy(&left.stdout).trim().to_string();
        ensure!(left.is_empty(), "processes left: {left}");
        let dirs = backend.block_on(conn.exec(&[
            "sh",
            "-c",
            "ls -A \"$HOME/.cache/senvakara\" 2>/dev/null || true",
        ]))?;
        println!(
            "  run folders left: {:?}",
            String::from_utf8_lossy(&dirs.stdout).trim()
        );
        backend.block_on(conn.exec(&["sh", "-c", &format!("rm -f \"{marker}\"")]))?;
        Ok(())
    })();
    let _ = ui.block_on(app.host_delete(id));
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    println!("== all checks passed");
    Ok(())
}
