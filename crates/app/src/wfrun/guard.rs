//! Anti-lockout for guarded modules (plan WF §4, ADR-031): SSH hardening and
//! the firewall. Before `apply.sh` the host gets a systemd timer that runs the
//! module's `rollback.sh` after a few minutes. Senvakara stops that timer only
//! after it signed in again on a new connection (on the new port when the
//! port moved) and ran the disarm through it. If it cannot sign in, the host
//! restores its own previous settings; if the apply or its check fails while
//! the old session still works, the rollback runs at once.
//!
//! The three root scripts below are fixed, reviewed text; only the unit name
//! (checked here and again in the script) and a path are passed as arguments.

use super::remote::{password_line, HostRun};
use super::RunCtx;
use crate::state::CmdResult;
use senvakara_adapters_ssh::{ExecEnd, ExecEvent, ExecLimits, Secret, SshConnection};
use senvakara_core::{CoreError, ErrorCode};
use std::time::Duration;
use tokio::sync::watch;

/// Root folder of guard state on the host; one folder per armed change.
const GUARD_ROOT: &str = "/var/lib/senvakara/guard";

/// Schedules the rollback. `$1` unit, `$2` seconds, `$3` the uploaded
/// rollback script. The timer runs `run.sh`, which leaves an empty
/// `<unit>.ran` marker (so the disarm can tell "stopped in time" from
/// "already undone"), restores, and removes the backup folder. Markers older
/// than a day are swept here.
const ARM: &str = r#"set -eu
unit=$1 secs=$2 src=$3
case $unit in sv-guard-*) ;; *) echo "bad unit name" >&2; exit 2 ;; esac
case $unit in *[!a-z0-9-]*) echo "bad unit name" >&2; exit 2 ;; esac
case $secs in '' | *[!0-9]*) echo "bad delay" >&2; exit 2 ;; esac
command -v systemd-run >/dev/null || { echo "systemd-run is missing" >&2; exit 3; }
dir=/var/lib/senvakara/guard/$unit
install -d -m 0700 /var/lib/senvakara /var/lib/senvakara/guard "$dir"
find /var/lib/senvakara/guard -maxdepth 1 -name 'sv-guard-*.ran' -mtime +1 -delete
install -m 0700 "$src" "$dir/rollback.sh"
printf '#!/bin/bash\ntouch %s.ran\n/bin/bash %s/rollback.sh\nrm -rf -- %s\n' "$dir" "$dir" "$dir" >"$dir/run.sh"
chmod 0700 "$dir/run.sh"
systemd-run --quiet --unit="$unit" --on-active="${secs}s" --timer-property=AccuracySec=1s /bin/bash "$dir/run.sh"
"#;

/// Stops the timer and forgets the backup. Exit 5: the rollback already ran.
const DISARM: &str = r#"set -u
unit=$1
case $unit in sv-guard-*) ;; *) echo "bad unit name" >&2; exit 2 ;; esac
case $unit in *[!a-z0-9-]*) echo "bad unit name" >&2; exit 2 ;; esac
dir=/var/lib/senvakara/guard/$unit
systemctl stop "$unit.timer" 2>/dev/null || true
if [ -e "$dir.ran" ]; then
  rm -f -- "$dir.ran"
  echo "the rollback already ran" >&2
  exit 5
fi
rm -rf -- "$dir"
"#;

/// Undoes the change now, through the session that still works.
const UNDO: &str = r#"set -u
unit=$1
case $unit in sv-guard-*) ;; *) echo "bad unit name" >&2; exit 2 ;; esac
case $unit in *[!a-z0-9-]*) echo "bad unit name" >&2; exit 2 ;; esac
dir=/var/lib/senvakara/guard/$unit
systemctl stop "$unit.timer" 2>/dev/null || true
if [ ! -e "$dir.ran" ] && [ -f "$dir/run.sh" ]; then
  /bin/bash "$dir/run.sh"
fi
rm -f -- "$dir.ran"
rm -rf -- "$dir"
"#;

/// A scheduled rollback on one host.
pub(super) struct Armed {
    pub unit: String,
    pub secs: u32,
}

/// The systemd unit of the `n`th guarded change of a run: lowercase letters,
/// digits and dashes only, short enough for any unit name limit.
pub(super) fn unit_name(run_id: &str, n: usize) -> String {
    let id: String = run_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .take(20)
        .collect();
    format!("sv-guard-{id}-{n}")
}

/// Where `apply.sh` keeps its backup for `unit` (`SV_GUARD_DIR`).
pub(super) fn guard_dir(unit: &str) -> String {
    format!("{GUARD_ROOT}/{unit}")
}

fn err(code: ErrorCode, message: impl Into<String>) -> CoreError {
    CoreError::new(code, message)
}

fn never_cancelled() -> watch::Receiver<bool> {
    watch::channel(false).1
}

/// Runs one of the fixed scripts above as root; returns its exit status and
/// what it printed on stderr (for the message).
async fn root_script(
    conn: &SshConnection,
    sudo: &Option<Secret>,
    script: &str,
    args: &[&str],
) -> CmdResult<(u32, String)> {
    let line = password_line(sudo).ok_or_else(|| {
        err(
            ErrorCode::Unauthorized,
            "enter the sudo password for this run",
        )
    })?;
    let mut argv: Vec<&str> = vec!["sudo", "-k", "-S", "-p", "", "--", "bash", "-c", script];
    argv.push("sv-guard");
    argv.extend(args);
    let mut stderr = Vec::new();
    let report = conn
        .exec_stream(
            &argv,
            Some(line.as_bytes()),
            ExecLimits {
                timeout: Duration::from_secs(60),
                drain: Duration::from_secs(2),
            },
            never_cancelled(),
            |event| {
                if let ExecEvent::Stderr(b) = event {
                    if stderr.len() < 4096 {
                        stderr.extend_from_slice(&b);
                    }
                }
            },
            || async {},
        )
        .await
        .map_err(|e| crate::state::to_core("run a guard step", e))?;
    let text = String::from_utf8_lossy(&stderr).trim().to_string();
    match report.end {
        ExecEnd::Exited(code) => Ok((code, text)),
        _ => Err(err(
            ErrorCode::HostOffline,
            "the host stopped answering during a guard step",
        )),
    }
}

/// Uploads the rollback script (with the same settings as the apply) and
/// schedules it. Nothing is applied when this fails.
pub(super) async fn arm(
    ctx: &RunCtx,
    host: &HostRun,
    unit: String,
    secs: u32,
    rollback: &str,
) -> CmdResult<Armed> {
    let path = format!("{}/{unit}-rollback.sh", host.dir);
    tokio::time::timeout(
        Duration::from_secs(30),
        senvakara_adapters_ssh::sftp::upload_bytes(&host.sftp, &path, rollback.as_bytes(), 0o700),
    )
    .await
    .map_err(|_| {
        err(
            ErrorCode::Timeout,
            "the host did not answer while receiving rollback.sh",
        )
    })?
    .map_err(|e| crate::state::to_core("upload the rollback script", e))?;
    let secs_text = secs.to_string();
    let (code, why) = root_script(&host.conn, &ctx.sudo, ARM, &[&unit, &secs_text, &path]).await?;
    if code != 0 {
        return Err(err(
            ErrorCode::InternalError,
            format!("the safety rollback could not be scheduled, so nothing was changed ({why})"),
        ));
    }
    ctx.log(format!(
        "Safety net: {} undoes this change in {secs} s unless Senvakara signs in again.",
        host.name
    ));
    Ok(Armed { unit, secs })
}

/// The apply or its check failed while the old session still works: undo
/// at once instead of waiting for the timer. Best effort; the timer is the
/// fallback.
pub(super) async fn undo_now(ctx: &RunCtx, host: &HostRun, armed: &Armed) {
    match root_script(&host.conn, &ctx.sudo, UNDO, &[&armed.unit]).await {
        Ok((0, _)) => ctx.log(format!("The previous settings on {} are back.", host.name)),
        _ => ctx.log(format!(
            "Could not undo the change right away; {} undoes it by itself within {} s.",
            host.name, armed.secs
        )),
    }
}

/// Signs in again on a new connection (on `port` when the SSH port moved)
/// and stops the rollback through it. Fails, leaving the rollback to run,
/// when the new sign-in does not work.
pub(super) async fn prove_and_disarm(
    ctx: &RunCtx,
    host_id: &str,
    host: &HostRun,
    armed: &Armed,
    port: Option<u16>,
) -> CmdResult<()> {
    let where_ = port.map(|p| format!(" on port {p}")).unwrap_or_default();
    ctx.log(format!(
        "Signing in to {}{where_} again to confirm the change...",
        host.name
    ));
    let fresh = tokio::time::timeout(
        Duration::from_secs(45),
        ctx.app.state.connect_fresh(host_id, port),
    )
    .await
    .map_err(|_| err(ErrorCode::Timeout, "no answer"))
    .and_then(|r| r);
    let fresh = match fresh {
        Ok(conn) => conn,
        Err(e) => {
            return Err(err(
                ErrorCode::HostOffline,
                format!(
                    "Senvakara could not sign in to {}{where_} again after the change ({}). The host restores its previous settings by itself within {} s; run Preview again after that.",
                    host.name, e.message, armed.secs
                ),
            ))
        }
    };
    let result = root_script(&fresh, &ctx.sudo, DISARM, &[&armed.unit]).await;
    let _ = fresh.close().await;
    let (code, why) = match result {
        Ok(done) => done,
        // Signing in worked; the old session can stop the timer as well.
        Err(_) => root_script(&host.conn, &ctx.sudo, DISARM, &[&armed.unit]).await?,
    };
    match code {
        0 => {
            ctx.log(format!("Signed in again; the safety rollback on {} is cancelled.", host.name));
            Ok(())
        }
        5 => Err(err(
            ErrorCode::Timeout,
            format!("The safety window ran out before Senvakara signed in again, so {} restored its previous settings.", host.name),
        )),
        _ => Err(err(
            ErrorCode::InternalError,
            format!("the safety rollback could not be cancelled ({why}); {} undoes the change within {} s", host.name, armed.secs),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_are_safe_for_systemd_and_the_scripts() {
        let name = unit_name("run-3F2a_9b/../x;rm -rf ~ 0123456789abcdef", 12);
        assert_eq!(name, "sv-guard-run3f2a9bxrmrf012345-12");
        assert!(name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert_eq!(guard_dir(&name), format!("/var/lib/senvakara/guard/{name}"));
    }

    #[test]
    fn the_guard_scripts_check_the_unit_name_themselves() {
        for script in [ARM, DISARM, UNDO] {
            assert!(script.contains("case $unit in *[!a-z0-9-]*)"));
            assert!(script.contains("case $unit in sv-guard-*)"));
        }
        // The timer marks that it ran before restoring anything.
        assert!(ARM.contains("touch %s.ran"));
    }
}
