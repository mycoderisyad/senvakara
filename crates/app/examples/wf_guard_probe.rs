//! End-to-end check of the Phase 2 server modules, the posture check and the
//! anti-lockout guard (plan WF §4/§5, ADR-031) against the throw-away test
//! host of WF-SPIKE-02 (`svtest` on 127.0.0.1:2222, password sign-in).
//!
//! It changes the test host: it moves the spike sshd to an extra port 2223,
//! briefly blocks new SSH connections with iptables to prove the timed
//! rollback, and, with `SV_PROBE_INSTALL=1`, creates the user `svprobe` and
//! installs unattended-upgrades, fail2ban and ufw (ufw is switched off again
//! at the end unless it was on before). Run it only on the WSL test distro,
//! with the owner's go-ahead:
//!   wsl -d Ubuntu-24.04 -u root -- bash tools/spikes/ssh-exec/setup-wsl.sh
//!   cargo run -p senvakara-app --example wf_guard_probe
//!   wsl -d Ubuntu-24.04 -u root -- bash tools/spikes/ssh-exec/teardown-wsl.sh

use anyhow::{bail, ensure, Context, Result};
use senvakara_app::workflow::{Binding, Edge, EdgeKind, Node, Permissions, Workflow};
use senvakara_app::{
    App, AppPaths, FlowEvent, HostUpsert, Protocol, Secret, Sink, SshAuth, WorkflowRunRequest,
};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// A public key whose private half was thrown away: only `authorized_keys`
/// handling is tested, nobody signs in with it.
const TEST_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINUR57ukeN61o0LWOkjKsvlIbiV/6JZ6vDK9eXS5si05 svprobe-test";
const NEW_PORT: u16 = 2223;

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

fn node(id: &str, module: &str, params: serde_json::Value) -> Node {
    Node {
        id: id.into(),
        module: module.into(),
        title: id.into(),
        x: 0.0,
        y: 0.0,
        params: serde_json::from_value(params).unwrap_or_default(),
        binding: None,
        scripts: BTreeMap::new(),
    }
}

/// `actions` on `host_id`, run in the order given.
fn graph(host_id: &str, actions: Vec<Node>) -> Workflow {
    let mut host = node("host", "host.ssh", json!({}));
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

type Steps = HashMap<String, (String, String)>;

fn run(ui: &tokio::runtime::Runtime, app: &Arc<App>, wf: &Workflow, sudo: &str) -> Result<Steps> {
    let saved = app.workflow_save(None, "Guard probe".into(), wf)?;
    let (tx, rx) = mpsc::channel();
    let sink: Sink<FlowEvent> = Arc::new(move |e| tx.send(e).is_ok());
    ui.block_on(app.workflow_run(
        WorkflowRunRequest {
            workflow_id: saved.id.clone(),
            sudo: Some(Secret::new(sudo.to_string())),
            serials: HashMap::new(),
        },
        sink,
    ))?;
    let started = Instant::now();
    let mut steps = HashMap::new();
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
                steps.insert(name, (status, detail.unwrap_or_default()));
            }
            Ok(FlowEvent::Log { text }) => println!("    {text}"),
            Ok(FlowEvent::Done { status, message }) => {
                println!(
                    "  done: {status} ({message}) in {:.0}s",
                    started.elapsed().as_secs_f64()
                );
                let _ = app.workflow_delete(&saved.id);
                return Ok(steps);
            }
            Err(mpsc::RecvTimeoutError::Timeout)
                if started.elapsed() > Duration::from_secs(1500) =>
            {
                bail!("run did not finish in 25 min")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => bail!("events ended: {e}"),
        }
    }
}

fn ok(steps: &Steps, name: &str) -> Result<()> {
    let (status, detail) = steps.get(name).cloned().unwrap_or_default();
    ensure!(status == "succeeded", "{name}: {status} {detail}");
    Ok(())
}

/// A root command on the test host through the pooled connection.
fn root(
    backend: &tokio::runtime::Runtime,
    app: &Arc<App>,
    id: &str,
    pw: &str,
    cmd: &str,
) -> Result<String> {
    let conn = backend.block_on(app.state.connection(id))?;
    let script = format!(
        "printf '%s\\n' {} | sudo -S -p '' -- bash -c {} 2>&1",
        shell(pw),
        shell(cmd)
    );
    let out = backend.block_on(conn.exec(&["sh", "-c", &script]))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn shell(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn posture(backend: &tokio::runtime::Runtime, app: &Arc<App>, id: &str) -> Result<()> {
    let view = backend.block_on(app.host_posture_check(id.to_string()))?;
    let (pass, all) = view.posture.score();
    println!(
        "  posture: Hardened {pass}/{all} (os {:?})",
        view.posture.os
    );
    for c in &view.posture.controls {
        println!("    {:<17} {:?}: {}", c.id, c.status, c.evidence);
    }
    Ok(())
}

fn main() -> Result<()> {
    let c = creds()?;
    let install = std::env::var("SV_PROBE_INSTALL").as_deref() == Ok("1");
    let dir = std::env::temp_dir().join(format!("senvakara-guardprobe-{}", std::process::id()));
    let paths = AppPaths::in_dir(&dir);
    let backend = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = App::start(paths, backend.handle().clone())?;
    let ui = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let host = app.host_create(HostUpsert {
        display_name: "Guard probe".into(),
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
    let result = (|| -> Result<()> {
        ui.block_on(app.host_set_password(id.clone(), Some(pw.clone())))?;
        let key = ui.block_on(app.host_check_key(id.clone()))?;
        ui.block_on(app.host_trust_key(id.clone(), key.presented))?;

        println!("== 1. posture check (read-only, no sudo)");
        posture(&backend, &app, &id)?;

        println!("== 2. turning passwords off is refused while Senvakara signs in with one");
        let refuse = graph(
            &id,
            vec![node("harden", "security.ssh_hardening", json!({}))],
        );
        let steps = run(&ui, &app, &refuse, &pw)?;
        let (status, detail) = steps["harden"].clone();
        ensure!(status == "failed", "harden: {status}");
        ensure!(detail.contains("exit status 3"), "{detail}");
        let left = root(
            &backend,
            &app,
            &id,
            &pw,
            "ls /var/lib/senvakara/guard 2>/dev/null; ls /etc/ssh/sshd_config.d",
        )?;
        println!("  after the refusal: {left}");
        ensure!(
            !left.contains("00-senvakara-hardening"),
            "the refusal changed sshd"
        );

        println!(
            "== 3. move SSH to port {NEW_PORT}: guarded, proven on a new connection, Hosts follows"
        );
        let mv = graph(
            &id,
            vec![node(
                "port",
                "security.ssh_hardening",
                json!({"disable_passwords": false, "port": NEW_PORT}),
            )],
        );
        let steps = run(&ui, &app, &mv, &pw)?;
        ok(&steps, "port")?;
        let moved = app.hosts_list()?.into_iter().find(|h| h.id == id).unwrap();
        ensure!(
            moved.ssh.as_ref().unwrap().port == NEW_PORT,
            "Hosts kept the old port"
        );
        let guards = root(
            &backend,
            &app,
            &id,
            &pw,
            "ls -A /var/lib/senvakara/guard; systemctl list-timers --all --no-legend 'sv-guard-*'",
        )?;
        ensure!(guards.is_empty(), "guard left behind: {guards}");
        println!("== 3b. same settings again: up to date, nothing armed");
        let steps = run(&ui, &app, &mv, &pw)?;
        ensure!(
            steps["port"].1 == "already up to date",
            "{:?}",
            steps["port"]
        );

        println!("== 4. a change that locks new sign-ins out is undone by the host itself");
        let mut lock = node(
            "lockout",
            "security.ssh_hardening",
            json!({"disable_passwords": false}),
        );
        lock.scripts.insert(
            "check.sh".into(),
            "[ -f /run/sv-probe-lock ] && exit 0 || exit 10\n".into(),
        );
        lock.scripts.insert(
            "apply.sh".into(),
            format!("touch /run/sv-probe-lock\niptables -I INPUT -p tcp --dport {NEW_PORT} -m conntrack --ctstate NEW -j REJECT\n"),
        );
        lock.scripts.insert(
            "rollback.sh".into(),
            format!("set +e\niptables -D INPUT -p tcp --dport {NEW_PORT} -m conntrack --ctstate NEW -j REJECT\nrm -f /run/sv-probe-lock\n"),
        );
        let lock_wf = graph(&id, vec![lock]);
        let steps = run(&ui, &app, &lock_wf, &pw)?;
        let (status, detail) = steps["lockout"].clone();
        ensure!(
            status == "failed" && detail.contains("could not sign in"),
            "{status} {detail}"
        );
        let waited = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(10));
            let state = root(
                &backend,
                &app,
                &id,
                &pw,
                "test -e /run/sv-probe-lock && echo locked || echo restored",
            )?;
            println!("  {:>3}s: {state}", waited.elapsed().as_secs());
            if state.ends_with("restored") {
                break;
            }
            ensure!(
                waited.elapsed() < Duration::from_secs(240),
                "no rollback after 240 s"
            );
        }
        let fresh = backend.block_on(app.state.connect_fresh(&id, None))?;
        backend.block_on(fresh.close())?;
        println!(
            "  a new connection works again after {}s",
            waited.elapsed().as_secs()
        );

        if install {
            println!("== 5. baseline modules (installs packages, creates svprobe, ufw on)");
            let ufw_before = root(
                &backend,
                &app,
                &id,
                &pw,
                "ufw status 2>/dev/null | head -n 1 || true",
            )?;
            let wf = graph(
                &id,
                vec![
                    node(
                        "user",
                        "security.sudo_user",
                        json!({"username": "svprobe", "public_key": TEST_KEY}),
                    ),
                    node("auto", "security.unattended_upgrades", json!({})),
                    node("sysctl", "security.sysctl_hardening", json!({})),
                    node("fw", "networking.firewall", json!({})),
                    node("f2b", "security.fail2ban", json!({})),
                ],
            );
            let steps = run(&ui, &app, &wf, &pw)?;
            let results: Vec<_> = ["user", "auto", "sysctl", "fw", "f2b"]
                .iter()
                .map(|s| (s, ok(&steps, s)))
                .collect();
            posture(&backend, &app, &id)?;
            println!("== 5b. again: everything up to date");
            let again = run(&ui, &app, &wf, &pw)?;
            if !ufw_before.contains("Status: active") {
                root(&backend, &app, &id, &pw, "ufw --force disable")?;
            }
            for (name, r) in results {
                r.with_context(|| format!("module {name}"))?;
            }
            for name in ["user", "auto", "sysctl", "fw", "f2b"] {
                ensure!(
                    again[name].1 == "already up to date",
                    "{name}: {:?}",
                    again[name]
                );
            }
        }
        Ok(())
    })();
    // Put the spike sshd back: drop the hardening drop-in, keep port 2222.
    let _ = root(&backend, &app, &id, &pw, "rm -f /etc/ssh/sshd_config.d/00-senvakara-hardening.conf; systemctl reload ssh; iptables -D INPUT -p tcp --dport 2223 -m conntrack --ctstate NEW -j REJECT 2>/dev/null; rm -f /run/sv-probe-lock; true");
    let _ = ui.block_on(app.host_delete(id));
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    println!("== all checks passed");
    Ok(())
}
