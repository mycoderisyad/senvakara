//! Host security posture (plan WF §5): the read-only check of
//! `senvakara-workflow`, run over the host's SSH connection as the signed-in
//! user (never sudo), with the summary cached per host in `host_posture`.
//! It runs only when the user asks (opening a workflow, Refresh, after a
//! run), never on a timer (AGENTS rule 8). Not a compliance audit.

use crate::state::{db_err, CmdResult};
use crate::App;
use senvakara_adapters_ssh::{ExecEnd, ExecEvent, ExecLimits};
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::PostureRecord;
use senvakara_workflow::{Posture, POSTURE_SCRIPT};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// The check reads files and asks systemd and apt; it never takes long.
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
/// More output than this is not a posture report.
const MAX_OUTPUT: usize = 64 * 1024;

/// A host's last posture with when it was taken.
#[derive(Debug, Clone, PartialEq)]
pub struct PostureView {
    pub host_id: String,
    /// UTC, ISO 8601.
    pub checked_at: String,
    pub age_secs: i64,
    pub posture: Posture,
}

fn view(record: PostureRecord) -> Option<PostureView> {
    let posture = serde_json::from_value(record.results).ok()?;
    Some(PostureView {
        host_id: record.host_id,
        checked_at: record.checked_at,
        age_secs: record.age_secs,
        posture,
    })
}

/// Every cached posture. One that no longer reads is left out.
pub(crate) fn host_postures(app: &App) -> CmdResult<Vec<PostureView>> {
    let records = app
        .state
        .db
        .lock()
        .unwrap()
        .list_postures()
        .map_err(|e| db_err("read postures", e))?;
    Ok(records.into_iter().filter_map(view).collect())
}

/// Runs the check on `host_id` (connecting when needed: the user asked) and
/// stores the result.
pub(crate) async fn host_posture_check(app: &Arc<App>, host_id: String) -> CmdResult<PostureView> {
    let conn = app.state.connection(&host_id).await?;
    let mut stdout: Vec<u8> = Vec::new();
    let report = conn
        .exec_stream(
            &["bash", "-c", POSTURE_SCRIPT, "sv-posture"],
            None,
            ExecLimits {
                timeout: CHECK_TIMEOUT,
                drain: Duration::from_secs(2),
            },
            watch::channel(false).1,
            |event| {
                if let ExecEvent::Stdout(bytes) = event {
                    if stdout.len() < MAX_OUTPUT {
                        stdout.extend_from_slice(&bytes);
                    }
                }
            },
            || async {},
        )
        .await
        .map_err(|e| crate::state::to_core("check the security posture", e))?;
    if !matches!(report.end, ExecEnd::Exited(_)) {
        return Err(CoreError::new(
            ErrorCode::Timeout,
            "the security check did not finish",
        ));
    }
    let posture = Posture::parse(&String::from_utf8_lossy(&stdout))
        .map_err(|e| CoreError::new(ErrorCode::InternalError, e))?;
    let json = serde_json::to_value(&posture)
        .map_err(|e| CoreError::internal(format!("encode posture: {e}")))?;
    let record = app
        .state
        .db
        .lock()
        .unwrap()
        .save_posture(&host_id, &json)
        .map_err(|e| db_err("save posture", e))?;
    Ok(PostureView {
        host_id,
        checked_at: record.checked_at,
        age_secs: record.age_secs,
        posture,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_cached_result_that_no_longer_reads_is_skipped() {
        let record = |results| PostureRecord {
            host_id: "h".into(),
            checked_at: "2026-11-27T10:00:00.000Z".into(),
            age_secs: 30,
            results,
        };
        assert!(view(record(json!({"unexpected": true}))).is_none());
        let good = Posture::parse("SV_POSTURE\troot_login\tpass\tPermitRootLogin no\n").unwrap();
        let v = view(record(serde_json::to_value(&good).unwrap())).unwrap();
        assert_eq!(v.posture, good);
        assert_eq!(v.age_secs, 30);
    }
}
