//! Prepares a data folder for checking the Workflows UI by hand or through the
//! Slint MCP server (WF-IMPL-01): registers the WF-SPIKE-02 test host from
//! `tools/spikes/ssh-exec/creds.local` and saves two workflows on it, "UI run"
//! (an idempotent marker plus a root check) and "UI stop" (a long script to
//! stop). The host's password goes to Credential Manager like any host's;
//! delete the host in the app afterwards to remove it.
//!
//! Run: cargo run -p senvakara-app --example wf_seed -- <data folder>
//! Clean up: the same with `--remove` after the folder name, before deleting
//! the folder (removes its SSH hosts and their stored passwords).

use anyhow::{Context, Result};
use senvakara_app::workflow::{Binding, Edge, EdgeKind, Node, Permissions, Workflow};
use senvakara_app::{App, AppPaths, HostUpsert, Protocol, SshAuth};
use std::collections::BTreeMap;

fn creds() -> Result<(String, u16, String, String)> {
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
    Ok((
        get("host")?,
        get("port")?.parse()?,
        get("user")?,
        get("password")?,
    ))
}

fn node(id: &str, module: &str, x: f32, y: f32) -> Node {
    Node {
        id: id.into(),
        module: module.into(),
        title: String::new(),
        x,
        y,
        params: BTreeMap::new(),
        binding: None,
        scripts: BTreeMap::new(),
    }
}

fn script(id: &str, title: &str, y: f32, check: Option<&str>, apply: &str, root: bool) -> Node {
    let mut n = node(id, "automation.custom_script", 320.0, y);
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

fn on_host(host_id: &str, actions: Vec<Node>, after: &[(&str, &str)]) -> Workflow {
    let mut host = node("host", "host.ssh", 0.0, 60.0);
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
        wf.edges.push(Edge {
            from: a.id.clone(),
            to: "host".into(),
            kind: EdgeKind::Target,
        });
    }
    for (a, b) in after {
        wf.edges.push(Edge {
            from: (*a).into(),
            to: (*b).into(),
            kind: EdgeKind::After,
        });
    }
    wf.nodes.push(host);
    wf.nodes.extend(actions);
    wf
}

fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .context("usage: wf_seed <data folder> [--remove]")?;
    let paths = AppPaths::in_dir(&dir);
    let _instance = senvakara_app::single_instance(&paths)?;
    let backend = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let app = App::start(paths, backend.handle().clone())?;
    // Clean up: delete the folder's SSH hosts and their stored passwords. Do
    // this before deleting the folder, or the passwords stay behind.
    if std::env::args().nth(2).as_deref() == Some("--remove") {
        for host in app.hosts_list()?.into_iter().filter(|h| !h.is_local) {
            backend.block_on(app.host_delete(host.id.clone()))?;
            println!("removed host {}", host.id);
        }
        return Ok(());
    }
    let (address, port, user, password) = creds()?;

    let host = app.host_create(HostUpsert {
        display_name: "WSL test host".into(),
        os_family: "linux".into(),
        protocol: Protocol::Ssh,
        address,
        port,
        username: user,
        auth: SshAuth::Password,
        group_name: None,
        tags: Vec::new(),
        startup_snippet: None,
    })?;
    let id = host.id.clone();
    // Everything on the backend runtime, as App's own methods do.
    backend.block_on(async {
        app.host_set_password(id.clone(), Some(password)).await?;
        let key = app.host_check_key(id.clone()).await?;
        // Loopback test host made by setup-wsl.sh.
        app.host_trust_key(id.clone(), key.presented).await?;
        let conn = app.state.connection(&id).await?;
        conn.exec(&["sh", "-c", "rm -f \"$HOME/.cache/sv-ui-marker\""])
            .await
            .map_err(|e| senvakara_app::CoreError::internal(e.to_string()))?;
        anyhow::Ok(())
    })?;

    let marker = "$HOME/.cache/sv-ui-marker";
    let run = on_host(
        &id,
        vec![
            script(
                "mark",
                "Make marker",
                0.0,
                Some(&format!("[ -f \"{marker}\" ] && exit 0 || exit 10\n")),
                &format!("touch \"{marker}\"\necho created as $(id -un)\n"),
                false,
            ),
            script(
                "root",
                "Root check",
                140.0,
                None,
                "test \"$(id -u)\" = 0\necho root ok\n",
                true,
            ),
        ],
        &[("mark", "root")],
    );
    let stop = on_host(
        &id,
        vec![script(
            "long",
            "Long script",
            60.0,
            None,
            "sleep 600 &\nfor i in $(seq 1 600); do echo tick $i; sleep 1; done\n",
            true,
        )],
        &[],
    );
    let a = app.workflow_save(None, "UI run".into(), &run)?;
    let b = app.workflow_save(None, "UI stop".into(), &stop)?;
    println!(
        "host {id}\nworkflow UI run {}\nworkflow UI stop {}",
        a.id, b.id
    );
    Ok(())
}
