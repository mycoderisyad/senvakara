//! End-to-end check of the Phase 3 engine (ADR-033) against the throw-away
//! test host of WF-SPIKE-02 (`svtest` on 127.0.0.1:2222, password sign-in,
//! password sudo): secret settings delivered on stdin through sudo and masked
//! in the log, module files in `$SV_MODULE_DIR`, health after an apply and on
//! demand, Remove, and a secret that is gone from Credential Manager.
//!
//! Install-free: bundled modules run with edited scripts that only touch
//! `/tmp/sv-probe` on the host. It stores one test secret in Credential
//! Manager and deletes it again. With `SV_PROBE_INSTALL=1` (owner's go-ahead
//! only) it then installs Docker Engine and runs the Redis and Adminer
//! modules for real, twice, and removes them (data stays).
//!   wsl -d Ubuntu-24.04 -u root -- bash tools/spikes/ssh-exec/setup-wsl.sh
//!   cargo run -p senvakara-app --example wf_service_probe

use anyhow::{bail, ensure, Context, Result};
use senvakara_app::workflow::{Binding, Edge, EdgeKind, HealthState, Node, Permissions, Workflow};
use senvakara_app::{
    App, AppPaths, FlowEvent, HostUpsert, Protocol, Secret, Sink, SshAuth, WorkflowRemoveRequest,
    WorkflowRunRequest,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Quotes, a command substitution, a second line and a trailing newline: all
/// must reach the script unchanged and never show in the log.
const TOKEN: &str = "probe-t0k'en \"$(id)\" ;rm -rf /x\nsecond-line-of-it\n";

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

fn node(id: &str, module: &str, params: serde_json::Value, scripts: &[(&str, String)]) -> Node {
    Node {
        id: id.into(),
        module: module.into(),
        title: id.into(),
        x: 0.0,
        y: 0.0,
        params: serde_json::from_value(params).unwrap_or_default(),
        binding: None,
        scripts: scripts
            .iter()
            .map(|(f, s)| (f.to_string(), s.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn graph(host_id: &str, actions: Vec<Node>) -> Workflow {
    let mut host = node("host", "host.ssh", json!({}), &[]);
    host.binding = Some(Binding::Host {
        host_id: host_id.into(),
    });
    let mut wf = Workflow {
        permissions: Permissions {
            root: true,
            shell: true,
            install: true,
            network: true,
        },
        ..Workflow::default()
    };
    for (i, a) in actions.iter().enumerate() {
        wf.edges.push(Edge {
            from: a.id.clone(),
            to: "host".into(),
            kind: EdgeKind::Target,
        });
        if i > 0 {
            wf.edges.push(Edge {
                from: actions[i - 1].id.clone(),
                to: a.id.clone(),
                kind: EdgeKind::After,
            });
        }
    }
    wf.nodes.push(host);
    wf.nodes.extend(actions);
    wf
}

/// Step results by node title, and every log line.
struct Ran {
    steps: HashMap<String, (String, String)>,
    lines: Vec<String>,
}

enum Start {
    Run,
    Remove(String),
}

fn run(
    ui: &tokio::runtime::Runtime,
    app: &Arc<App>,
    wf_id: &str,
    sudo: &str,
    start: Start,
) -> Result<Ran> {
    let (tx, rx) = mpsc::channel();
    let sink: Sink<FlowEvent> = Arc::new(move |e| tx.send(e).is_ok());
    let sudo = Some(Secret::new(sudo.to_string()));
    match start {
        Start::Run => ui.block_on(app.workflow_run(
            WorkflowRunRequest {
                workflow_id: wf_id.into(),
                sudo,
                serials: HashMap::new(),
            },
            sink,
        ))?,
        Start::Remove(node_id) => ui.block_on(app.workflow_remove(
            WorkflowRemoveRequest {
                workflow_id: wf_id.into(),
                node_id,
                sudo,
            },
            sink,
        ))?,
    };
    let started = Instant::now();
    let mut ran = Ran {
        steps: HashMap::new(),
        lines: Vec::new(),
    };
    loop {
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
                ran.steps.insert(name, (status, detail.unwrap_or_default()));
            }
            Ok(FlowEvent::Log { text }) => {
                println!("    {text}");
                ran.lines.push(text);
            }
            Ok(FlowEvent::Done { status, message }) => {
                println!(
                    "  done: {status} ({message}) in {:.0}s",
                    started.elapsed().as_secs_f64()
                );
                return Ok(ran);
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() > Duration::from_secs(1800) =>
            {
                bail!("run did not finish in 30 min")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => bail!("events ended: {e}"),
        }
    }
}

fn status(ran: &Ran, name: &str) -> (String, String) {
    ran.steps.get(name).cloned().unwrap_or_default()
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Edited scripts for a node that marks `/tmp/sv-probe/<name>.ok`.
fn marker_scripts(name: &str, apply_extra: &str, health: &str) -> Vec<(&'static str, String)> {
    let mark = format!("/tmp/sv-probe/{name}.ok");
    vec![
        (
            "check.sh",
            format!("if [ -f {mark} ]; then echo 'marker present'; exit 0; fi\necho 'no marker'\nexit 10\n"),
        ),
        (
            "apply.sh",
            format!("[ \"$(id -u)\" = 0 ] || {{ echo 'not root'; exit 3; }}\nmkdir -p /tmp/sv-probe\n{apply_extra}\ntouch {mark}\n"),
        ),
        ("health.sh", health.to_string()),
        ("remove.sh", format!("rm -f {mark}\necho 'marker removed; nothing else to keep'\n")),
    ]
}

fn main() -> Result<()> {
    let c = creds()?;
    let install = std::env::var("SV_PROBE_INSTALL").as_deref() == Ok("1");
    let dir = std::env::temp_dir().join(format!("senvakara-svcprobe-{}", std::process::id()));
    let paths = AppPaths::in_dir(&dir);
    let backend = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = App::start(paths, backend.handle().clone())?;
    let ui = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let host = app.host_create(HostUpsert {
        display_name: "Service probe".into(),
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
    let pw = c.password.clone();
    let mut reference: Option<String> = None;
    let mut saved: Vec<String> = Vec::new();
    let result = (|| -> Result<()> {
        ui.block_on(app.host_set_password(id.clone(), Some(pw.clone())))?;
        let key = ui.block_on(app.host_check_key(id.clone()))?;
        ui.block_on(app.host_trust_key(id.clone(), key.presented))?;

        println!("== 1. a secret setting is stored; the graph keeps only its reference");
        let r = ui.block_on(app.workflow_secret_put(Secret::new(TOKEN.to_string())))?;
        ensure!(r.starts_with("secret-"), "reference {r}");
        reference = Some(r.clone());

        // The script checks the value byte for byte (as hex, so the file never holds it),
        // that the script file and every process's argv do not hold it, then prints it.
        let tunnel_apply = format!(
            "got=$(printf '%s' \"$SV_TOKEN\" | od -An -tx1 -v | tr -d ' \\n')\n\
             [ \"$got\" = '{}' ] || {{ echo \"token differs: $got\"; exit 4; }}\n\
             case \"$(cat \"$0\")\" in *\"$SV_TOKEN\"*) echo 'token in the script file'; exit 5;; esac\n\
             case \"$(ps -eo args)\" in *second-line-of-it*) echo 'token in argv'; exit 6;; esac\n\
             echo \"leak test: $SV_TOKEN\"",
            hex(TOKEN)
        );
        let tunnel_health = "[ -z \"${SV_TOKEN:-}\" ] || { echo 'SV_HEALTH degraded the secret reached health.sh'; exit 0; }\n\
             [ \"$(id -u)\" != 0 ] || { echo 'SV_HEALTH degraded health.sh ran as root'; exit 0; }\n\
             echo \"SV_HEALTH running probe health, wait $SV_HEALTH_WAIT s\"\n";
        let files_apply = "grep -q '^name: sv-adminer' \"$SV_MODULE_DIR/compose.yml\" || { echo 'compose.yml missing'; exit 7; }\n\
             echo \"module files in $SV_MODULE_DIR: $(ls \"$SV_MODULE_DIR\")\"";
        let files_health =
            "if [ -f /tmp/sv-probe/files.ok ]; then echo 'SV_HEALTH running marker there'; \
             else echo 'SV_HEALTH stopped no marker'; fi\n";
        let wf = graph(
            &id,
            vec![
                node(
                    "tunnel",
                    "networking.cloudflared",
                    json!({ "token": r }),
                    &marker_scripts("tunnel", &tunnel_apply, tunnel_health),
                ),
                node(
                    "files",
                    "app.adminer",
                    json!({}),
                    &marker_scripts("files", files_apply, files_health),
                ),
            ],
        );
        let wf_id = app.workflow_save(None, "Service probe".into(), &wf)?.id;
        saved.push(wf_id.clone());

        println!("== 2. first run: secret through sudo stdin, module files, health after apply");
        let ran = run(&ui, &app, &wf_id, &pw, Start::Run)?;
        for name in ["tunnel", "files"] {
            let (s, d) = status(&ran, name);
            ensure!(
                s == "succeeded" && d.starts_with("applied; Running"),
                "{name}: {s} {d}"
            );
        }
        ensure!(
            status(&ran, "tunnel").1.contains("wait 90 s"),
            "health after apply gets the 90 s wait"
        );
        let leaked = ran
            .lines
            .iter()
            .any(|l| l.contains("second-line-of-it") || l.contains("probe-t0k"));
        ensure!(!leaked, "the secret shows in the run log");
        ensure!(
            ran.lines.iter().any(|l| l.contains("leak test: [secret]")),
            "the masked line is missing"
        );
        let log_dir = dir.join("runs");
        let mut on_disk = String::new();
        for run_dir in std::fs::read_dir(&log_dir)? {
            for f in std::fs::read_dir(run_dir?.path())? {
                on_disk.push_str(&std::fs::read_to_string(f?.path()).unwrap_or_default());
            }
        }
        ensure!(
            !on_disk.contains("second-line-of-it") && on_disk.contains("[secret]"),
            "the log file holds the secret"
        );

        println!("== 3. health on demand: one exec as the signed-in user");
        let health = ui.block_on(app.workflow_health(wf_id.clone(), id.clone()))?;
        println!("  {health:?}");
        ensure!(health.len() == 2, "two service nodes");
        ensure!(
            health.iter().all(|(_, h)| h.state == HealthState::Running
                && h.detail.contains("wait 0 s")
                || h.detail == "marker there"),
            "{health:?}"
        );

        println!("== 4. second run: both already up to date");
        let ran = run(&ui, &app, &wf_id, &pw, Start::Run)?;
        for name in ["tunnel", "files"] {
            let (s, d) = status(&ran, name);
            ensure!(
                s == "succeeded" && d == "already up to date",
                "{name}: {s} {d}"
            );
        }

        println!("== 5. Remove of one node runs its remove.sh only");
        let ran = run(&ui, &app, &wf_id, &pw, Start::Remove("files".into()))?;
        let (s, d) = status(&ran, "files");
        ensure!(
            s == "succeeded" && d == "removed; health now: Stopped",
            "files: {s} {d}"
        );
        ensure!(ran.steps.len() == 1, "only the removed node ran");
        let health = ui.block_on(app.workflow_health(wf_id.clone(), id.clone()))?;
        let files = health
            .iter()
            .find(|(n, _)| n == "files")
            .map(|(_, h)| h.state);
        ensure!(files == Some(HealthState::Stopped), "{health:?}");

        println!("== 6. a service still stopped after its apply fails the step");
        let dead = graph(
            &id,
            vec![node(
                "dead",
                "app.adminer",
                json!({}),
                &marker_scripts("dead", "true", "echo 'SV_HEALTH stopped nothing listens'\n"),
            )],
        );
        let dead_id = app
            .workflow_save(None, "Service probe dead".into(), &dead)?
            .id;
        saved.push(dead_id.clone());
        let ran = run(&ui, &app, &dead_id, &pw, Start::Run)?;
        let (s, d) = status(&ran, "dead");
        ensure!(
            s == "failed" && d.contains("service is stopped"),
            "dead: {s} {d}"
        );

        println!("== 7. a secret gone from Credential Manager stops the step before it runs");
        ui.block_on(app.workflow_secret_delete(r.clone()))?;
        reference = None;
        let ran = run(&ui, &app, &wf_id, &pw, Start::Run)?;
        let (s, d) = status(&ran, "tunnel");
        ensure!(
            s == "failed" && d.contains("no longer in Credential Manager"),
            "tunnel: {s} {d}"
        );

        if install {
            println!("== 8. real installs: Docker Engine, Redis, Adminer (owner's go-ahead)");
            install_probe(&ui, &app, &id, &pw, &mut saved)?;
        }
        Ok(())
    })();

    // Clean up whatever happened above.
    if let Some(r) = reference {
        let _ = ui.block_on(app.workflow_secret_delete(r));
    }
    for w in saved {
        let _ = app.workflow_delete(&w);
    }
    let conn = backend.block_on(app.state.connection(&id));
    if let Ok(conn) = conn {
        let script = format!(
            "printf '%s\\n' '{}' | sudo -S -p '' -- rm -rf /tmp/sv-probe",
            pw.replace('\'', r"'\''")
        );
        let _ = backend.block_on(conn.exec(&["sh", "-c", &script]));
    }
    let _ = ui.block_on(app.host_delete(id));
    let _ = std::fs::remove_dir_all(&dir);
    match &result {
        Ok(()) => println!("ALL CHECKS PASSED"),
        Err(e) => println!("FAILED: {e:#}"),
    }
    result
}

/// Real modules, twice each, then Remove. Docker stays installed afterwards.
fn install_probe(
    ui: &tokio::runtime::Runtime,
    app: &Arc<App>,
    id: &str,
    pw: &str,
    saved: &mut Vec<String>,
) -> Result<()> {
    let wf = graph(
        id,
        vec![
            node("docker", "runtime.docker", json!({}), &[]),
            node("redis", "database.redis", json!({}), &[]),
            node("adminer", "app.adminer", json!({}), &[]),
        ],
    );
    let wf_id = app
        .workflow_save(None, "Service probe installs".into(), &wf)?
        .id;
    saved.push(wf_id.clone());
    for round in 1..=2 {
        println!("  -- round {round}");
        let ran = run(ui, app, &wf_id, pw, Start::Run)?;
        for name in ["docker", "redis", "adminer"] {
            let (s, d) = status(&ran, name);
            ensure!(s == "succeeded", "{name}: {s} {d}");
            if round == 2 {
                ensure!(d == "already up to date", "{name} round 2: {d}");
            }
        }
    }
    let health = ui.block_on(app.workflow_health(wf_id.clone(), id.to_string()))?;
    println!("  health: {health:?}");
    ensure!(
        health.iter().all(|(_, h)| h.state == HealthState::Running),
        "{health:?}"
    );
    for name in ["adminer", "redis"] {
        let ran = run(ui, app, &wf_id, pw, Start::Remove(name.into()))?;
        let (s, d) = status(&ran, name);
        ensure!(s == "succeeded", "remove {name}: {s} {d}");
    }
    Ok(())
}
