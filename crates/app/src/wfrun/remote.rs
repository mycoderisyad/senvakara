//! Host actions over SSH (ADR-030). Per host and run: one private folder
//! `~/.cache/senvakara/run-<id>/` (0700), one sudo check. Per action: write
//! the settings as `export SV_*` lines plus the script into a new 0700 file,
//! start it in its own process group with stdin from /dev/null, stream its
//! output, and on cancel or timeout kill the whole group from a second exec.
//!
//! An action with `check.sh` is skipped when the check says nothing needs
//! doing (exit 0), applied when it says so (exit 10), and checked again after
//! `apply.sh`, which must leave it at exit 0.

use super::log::NodeLog;
use super::{Outcome, RunCtx};
use crate::state::CmdResult;
use crate::workflows::catalog;
use senvakara_adapters_ssh::sftp::{ensure_private_dir, remove, upload_bytes};
use senvakara_adapters_ssh::{ExecEnd, ExecEvent, ExecLimits, Secret, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_workflow::{preamble, Binding, CheckedNode, ParamValue};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// Starts the script in a new session (its pid is the process group), writes
/// that group to the pidfile, and runs the script with stdin from /dev/null,
/// so whatever sudo did not read (the password, when it asked for none)
/// never reaches the script. `$1` pidfile, `$2` script. Reviewed text, not
/// built from input (ADR-030).
const LAUNCHER: &str = r#"echo $$ > "$1" || exit 97; exec bash -euo pipefail "$2" </dev/null"#;

/// Ends a run's process group: TERM, up to 3 s, then KILL. Refuses a pgid
/// that is not a plain number > 1 (`kill -- -1` would reach every process).
const KILL_GROUP: &str = r#"pg=$(cat -- "$1" 2>/dev/null)
case $pg in ''|*[!0-9]*) echo "no process group" >&2; exit 2;; esac
[ "$pg" -gt 1 ] || { echo "bad process group" >&2; exit 2; }
kill -TERM -- "-$pg" 2>/dev/null || exit 0
for i in $(seq 1 30); do
  kill -0 -- "-$pg" 2>/dev/null || exit 0
  sleep 0.1
done
kill -KILL -- "-$pg" 2>/dev/null || true
"#;

/// Exit status of `check.sh` meaning "something needs doing".
const CHECK_NEEDS_APPLY: u32 = 10;
/// Output is still collected this long after a stop.
const DRAIN: Duration = Duration::from_secs(10);
/// Longest wait for one SFTP step (open, folder, upload, cleanup).
const SFTP_TIMEOUT: Duration = Duration::from_secs(30);

const SUDO: [&str; 6] = ["sudo", "-k", "-S", "-p", "", "--"];

/// One host as prepared for this run.
struct HostRun {
    conn: Arc<SshConnection>,
    sftp: senvakara_adapters_ssh::SftpSession,
    dir: String,
    /// `Some(true)` once the sudo password was accepted on this host.
    sudo_ok: Option<bool>,
    name: String,
}

#[derive(Default)]
pub(crate) struct Hosts {
    by_id: HashMap<String, HostRun>,
    scripts: usize,
}

impl Hosts {
    /// Removes each host's run folder. Best effort: a folder left behind holds
    /// only scripts, never secrets.
    pub async fn clean_up(&mut self, ctx: &RunCtx) {
        for host in self.by_id.values() {
            match tokio::time::timeout(SFTP_TIMEOUT, remove(&host.sftp, &host.dir)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => ctx.log(format!(
                    "Could not remove {} on {}: {e}",
                    host.dir, host.name
                )),
                Err(_) => ctx.log(format!(
                    "{} did not answer; {} was left behind",
                    host.name, host.dir
                )),
            }
        }
        self.by_id.clear();
    }
}

fn err(code: ErrorCode, message: impl Into<String>) -> CoreError {
    CoreError::new(code, message)
}

async fn prepare<'a>(
    ctx: &RunCtx,
    by_id: &'a mut HashMap<String, HostRun>,
    host_id: &str,
) -> CmdResult<&'a mut HostRun> {
    if !by_id.contains_key(host_id) {
        let conn = ctx.app.state.connection(host_id).await?;
        let name = ctx
            .app
            .state
            .db
            .lock()
            .unwrap()
            .get_host(host_id)
            .map(|h| h.display_name)
            .unwrap_or_else(|_| host_id.to_string());
        // A stalled connection must fail the step, not hold the run.
        let setup = async {
            let sftp = conn
                .sftp()
                .await
                .map_err(|e| crate::state::to_core("open SFTP", e))?;
            let home = sftp.canonicalize(".").await.map_err(|e| {
                err(
                    ErrorCode::InternalError,
                    format!("find the home folder: {e}"),
                )
            })?;
            let dir = format!("{home}/.cache/senvakara/run-{}", ctx.run_id);
            ensure_private_dir(&sftp, &dir, 0o700)
                .await
                .map_err(|e| crate::state::to_core("create the run folder", e))?;
            CmdResult::Ok((sftp, dir))
        };
        let (sftp, dir) = tokio::time::timeout(SFTP_TIMEOUT, setup)
            .await
            .map_err(|_| {
                err(
                    ErrorCode::Timeout,
                    format!("{name} did not answer while preparing the run folder"),
                )
            })??;
        by_id.insert(
            host_id.to_string(),
            HostRun {
                conn,
                sftp,
                dir,
                sudo_ok: None,
                name,
            },
        );
    }
    Ok(by_id.get_mut(host_id).expect("just inserted"))
}

fn password_line(sudo: &Option<Secret>) -> Option<Secret> {
    sudo.as_ref()
        .map(|p| Secret::new(format!("{}\n", p.as_str())))
}

fn never_cancelled() -> watch::Receiver<bool> {
    // The sender lives in the returned receiver's channel only as long as the
    // receiver: `exec_stream` treats a closed switch as "never cancelled".
    watch::channel(false).1
}

/// Checks the sudo password once per host with a no-op.
async fn check_sudo(ctx: &RunCtx, host: &mut HostRun) -> CmdResult<()> {
    if host.sudo_ok == Some(true) {
        return Ok(());
    }
    let line = password_line(&ctx.sudo).ok_or_else(|| {
        err(
            ErrorCode::Unauthorized,
            "enter the sudo password for this run",
        )
    })?;
    let mut argv: Vec<&str> = SUDO.to_vec();
    argv.push("true");
    let report = host
        .conn
        .exec_stream(
            &argv,
            Some(line.as_bytes()),
            ExecLimits {
                timeout: Duration::from_secs(20),
                drain: Duration::from_secs(2),
            },
            never_cancelled(),
            |_| {},
            || async {},
        )
        .await
        .map_err(|e| crate::state::to_core("check sudo", e))?;
    if report.end == ExecEnd::Exited(0) {
        host.sudo_ok = Some(true);
        Ok(())
    } else {
        host.sudo_ok = Some(false);
        Err(err(
            ErrorCode::Unauthorized,
            format!("{} did not accept the sudo password", host.name),
        ))
    }
}

/// The text of one script file: the node's edit, else the bundled file.
fn script_text(node_id: &str, ctx: &RunCtx, module: &str, file: &str) -> Option<String> {
    let edited = ctx
        .graph
        .node(node_id)
        .and_then(|n| n.scripts.get(file))
        .filter(|s| !s.trim().is_empty())
        .cloned();
    edited.or_else(|| {
        catalog()
            .get(module)
            .and_then(|m| m.files.get(file))
            .cloned()
    })
}

pub(crate) async fn run_node(
    ctx: &RunCtx,
    hosts: &mut Hosts,
    node: &CheckedNode,
) -> CmdResult<Outcome> {
    let Binding::Host { host_id } = &node.target else {
        return Err(err(
            ErrorCode::InvalidInput,
            "this step runs on an SSH host",
        ));
    };
    if node
        .params
        .values()
        .any(|v| matches!(v, ParamValue::SecretRef(_)))
    {
        return Err(err(
            ErrorCode::InvalidInput,
            "settings kept in Credential Manager are not supported yet",
        ));
    }
    let head =
        preamble(&node.params, &BTreeMap::new()).map_err(|e| err(ErrorCode::InvalidInput, e))?;
    let timeout = Duration::from_secs(
        catalog()
            .get(&node.module)
            .map_or(600, |m| m.manifest.timeout_secs.into()),
    );
    let apply = script_text(&node.id, ctx, &node.module, "apply.sh")
        .ok_or_else(|| err(ErrorCode::InvalidInput, "this step has no apply script"))?;
    let check = script_text(&node.id, ctx, &node.module, "check.sh");

    let Hosts { by_id, scripts } = hosts;
    let host = prepare(ctx, by_id, host_id).await?;
    if node.needs_root {
        check_sudo(ctx, host).await?;
    }
    let title = ctx.title(&node.id);
    let mut script = |file: &'static str| {
        *scripts += 1;
        (file, *scripts)
    };

    if let Some(check) = &check {
        let (file, n) = script("check.sh");
        match run_script(ctx, host, node, file, n, &head, check, &title, timeout).await? {
            0 => {
                return Ok(Outcome {
                    detail: "already up to date".into(),
                    changed: Some(false),
                    exit_code: Some(0),
                })
            }
            CHECK_NEEDS_APPLY => {}
            code => {
                return Err(err(
                    ErrorCode::InternalError,
                    format!("check.sh failed with exit status {code}"),
                ))
            }
        }
    }
    let (file, n) = script("apply.sh");
    let code = run_script(ctx, host, node, file, n, &head, &apply, &title, timeout).await?;
    if code != 0 {
        return Err(err(
            ErrorCode::InternalError,
            format!("apply.sh failed with exit status {code}"),
        ));
    }
    if let Some(check) = &check {
        let (file, n) = script("check.sh");
        let code = run_script(ctx, host, node, file, n, &head, check, &title, timeout).await?;
        if code != 0 {
            return Err(err(
                ErrorCode::InternalError,
                format!("apply.sh finished but check.sh still reports work to do (exit {code})"),
            ));
        }
    }
    Ok(Outcome {
        detail: "applied".into(),
        changed: Some(true),
        exit_code: Some(0),
    })
}

/// Uploads and runs one script; returns its exit status.
#[allow(clippy::too_many_arguments)]
async fn run_script(
    ctx: &RunCtx,
    host: &mut HostRun,
    node: &CheckedNode,
    file: &str,
    n: usize,
    head: &str,
    body: &str,
    title: &str,
    timeout: Duration,
) -> CmdResult<u32> {
    let stem = format!("{}-{n}-{}", node.id, file.trim_end_matches(".sh"));
    let path = format!("{}/{stem}.sh", host.dir);
    let pidfile = format!("{}/{stem}.pgid", host.dir);
    let text = format!("{head}{body}");
    tokio::time::timeout(
        SFTP_TIMEOUT,
        upload_bytes(&host.sftp, &path, text.as_bytes(), 0o700),
    )
    .await
    .map_err(|_| {
        err(
            ErrorCode::Timeout,
            format!("{} did not answer while receiving {file}", host.name),
        )
    })?
    .map_err(|e| crate::state::to_core("upload the script", e))?;

    let mut argv: Vec<&str> = Vec::new();
    if node.needs_root {
        argv.extend(SUDO);
    }
    argv.extend([
        "setsid", "-w", "bash", "-c", LAUNCHER, "sv-run", &pidfile, &path,
    ]);
    let stdin = if node.needs_root {
        password_line(&ctx.sudo)
    } else {
        None
    };

    ctx.log(format!("[{title}] {file} on {}", host.name));
    let log_dir = ctx.app.paths.data_dir.join("runs").join(&ctx.run_id);
    let mut log = NodeLog::new(&log_dir, &node.id, title, |line| ctx.log(line));
    let conn = host.conn.clone();
    let kill_pw = if node.needs_root {
        password_line(&ctx.sudo)
    } else {
        None
    };
    let root = node.needs_root;
    let kill_pidfile = pidfile.clone();
    let report = host
        .conn
        .exec_stream(
            &argv,
            stdin.as_ref().map(|s| s.as_bytes()),
            ExecLimits {
                timeout,
                drain: DRAIN,
            },
            ctx.cancel_rx.clone(),
            |event| match event {
                ExecEvent::Stdout(b) => log.push(false, &b),
                ExecEvent::Stderr(b) => log.push(true, &b),
            },
            move || async move {
                let mut argv: Vec<&str> = Vec::new();
                if root {
                    argv.extend(SUDO);
                }
                argv.extend(["bash", "-c", KILL_GROUP, "sv-kill", &kill_pidfile]);
                let _ = conn
                    .exec_stream(
                        &argv,
                        kill_pw.as_ref().map(|s| s.as_bytes()),
                        ExecLimits {
                            timeout: Duration::from_secs(15),
                            drain: Duration::from_secs(2),
                        },
                        never_cancelled(),
                        |_| {},
                        || async {},
                    )
                    .await;
            },
        )
        .await
        .map_err(|e| crate::state::to_core("start the script", e))?;
    log.finish();
    match report.end {
        ExecEnd::Exited(code) => Ok(code),
        ExecEnd::Signalled(signal) => Err(err(
            ErrorCode::InternalError,
            format!("{file} was stopped by signal {signal}"),
        )),
        ExecEnd::TimedOut => Err(err(
            ErrorCode::Timeout,
            format!("{file} took longer than {} s and was stopped", timeout.as_secs()),
        )),
        ExecEnd::Cancelled => Err(err(ErrorCode::Cancelled, "cancelled by user")),
        ExecEnd::Lost => Err(err(
            ErrorCode::HostOffline,
            format!(
                "the connection to {} was lost during {file}; what happened on the host is unknown, so it is not retried",
                host.name
            ),
        )),
    }
}
