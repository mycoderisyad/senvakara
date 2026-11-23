//! Run history text: stored raw errors worded for people, local times, status marks and rows.

use senvakara_app::{CoreError, ErrorCode, RunRecord};
use serde_json::Value;

use crate::inventory::friendly_with;
use crate::RunRow;

/// Turn a stored raw error ("InvalidInput: step 2: ...", "HOST_OFFLINE: ...") into the words
/// the rest of the app uses. Text that is not such an error is returned as it is.
pub fn friendly_raw(text: &str) -> String {
    const CODES: [(&str, &str, ErrorCode); 12] = [
        (
            "Unauthenticated",
            "UNAUTHENTICATED",
            ErrorCode::Unauthenticated,
        ),
        ("Unauthorized", "UNAUTHORIZED", ErrorCode::Unauthorized),
        ("HostOffline", "HOST_OFFLINE", ErrorCode::HostOffline),
        ("DeviceOffline", "DEVICE_OFFLINE", ErrorCode::DeviceOffline),
        (
            "AdbUnauthorized",
            "ADB_UNAUTHORIZED",
            ErrorCode::AdbUnauthorized,
        ),
        ("RouteStale", "ROUTE_STALE", ErrorCode::RouteStale),
        (
            "UnsupportedCapability",
            "UNSUPPORTED_CAPABILITY",
            ErrorCode::UnsupportedCapability,
        ),
        ("Timeout", "TIMEOUT", ErrorCode::Timeout),
        ("Cancelled", "CANCELLED", ErrorCode::Cancelled),
        ("Conflict", "CONFLICT", ErrorCode::Conflict),
        ("InvalidInput", "INVALID_INPUT", ErrorCode::InvalidInput),
        ("InternalError", "INTERNAL_ERROR", ErrorCode::InternalError),
    ];
    let text = text.trim();
    let Some((code, message)) = text.split_once(':') else {
        return text.to_string();
    };
    match CODES
        .iter()
        .find(|(camel, upper, _)| code == *camel || code == *upper)
    {
        Some((_, _, code)) => friendly_with(
            &CoreError::new(*code, message.trim()),
            "It is already running or was changed in the meantime.",
        ),
        None => text.to_string(),
    }
}

/// "2026-11-22 14:05" in local time, from the UTC time the database stores.
pub fn when_text(created_at: &str) -> String {
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S"));
    match parsed {
        Ok(time) => time
            .and_utc()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => created_at.to_string(),
    }
}

/// (tone, word) for the status mark of a run.
pub fn run_state(status: &str) -> (&'static str, &'static str) {
    match status {
        "succeeded" => ("live", "Succeeded"),
        "running" | "pending" => ("busy", "Running"),
        "cancelled" => ("idle", "Cancelled"),
        _ => ("attention", "Failed"),
    }
}

/// Whether a run is a deploy the user started from the Deploy card. A deploy step inside a
/// recipe is stored as a run of its own, marked `embedded`; it belongs to the recipe.
pub fn is_standalone_deploy(run: &RunRecord) -> bool {
    run.recipe_id.is_none() && run.summary.get("embedded").and_then(Value::as_bool) != Some(true)
}

/// One history row. `what` is the summary field of the middle column ("mode" or "recipe").
pub fn run_row(run: &RunRecord, what: &str) -> RunRow {
    let field = |key: &str| {
        run.summary
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let (tone, word) = run_state(&run.status);
    RunRow {
        when: when_text(&run.created_at).into(),
        // "Name (host · serial)" as stored; the dot is spelled out as a comma.
        target: field("target").replace(" · ", ", ").into(),
        what: field(what).into(),
        tone: tone.into(),
        state_text: word.into(),
        result: friendly_raw(&field("result")).into(),
        duration: run
            .summary
            .get("seconds")
            .and_then(Value::as_u64)
            .map(|seconds| format!("{seconds} s"))
            .unwrap_or_default()
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn stored_raw_errors_are_worded_for_people() {
        let worded = friendly_raw("InvalidInput: step 1: invalid package name");
        assert!(worded.starts_with("Something in the request is not valid."));
        assert!(worded.contains("step 1: invalid package name"));
        assert!(friendly_raw("DEVICE_OFFLINE: gone").starts_with("The phone is not connected"));
        assert!(friendly_raw("AdbUnauthorized: ").starts_with("The phone has not allowed"));
        // Not an error code: left alone.
        assert_eq!(
            friendly_raw("Installed and launched: dev.x.app"),
            "Installed and launched: dev.x.app"
        );
        assert_eq!(friendly_raw("3 steps done"), "3 steps done");
        assert_eq!(friendly_raw(""), "");
    }

    #[test]
    fn a_deploy_step_inside_a_recipe_is_not_a_standalone_deploy() {
        let mut run = RunRecord {
            id: "run_2".into(),
            recipe_id: None,
            status: "succeeded".into(),
            created_at: String::new(),
            finished_at: None,
            summary: json!({ "mode": "debug" }),
            steps: Vec::new(),
        };
        assert!(is_standalone_deploy(&run));
        run.summary = json!({ "mode": "debug", "embedded": true });
        assert!(!is_standalone_deploy(&run));
        run.summary = json!({ "mode": "debug" });
        run.recipe_id = Some("rec_1".into());
        assert!(!is_standalone_deploy(&run));
    }

    #[test]
    fn history_rows_read_the_stored_summary() {
        let run = RunRecord {
            id: "run_1".into(),
            recipe_id: Some("rec_1".into()),
            status: "failed".into(),
            created_at: "not a time".into(),
            finished_at: None,
            summary: json!({
                "recipe": "Smoke test",
                "target": "Redmi (RAFGT · abc123)",
                "result": "Timeout: build took too long",
                "seconds": 42,
            }),
            steps: Vec::new(),
        };
        let row = run_row(&run, "recipe");
        assert_eq!(row.when, "not a time");
        assert_eq!(row.target, "Redmi (RAFGT, abc123)");
        assert_eq!(row.what, "Smoke test");
        assert_eq!(row.state_text, "Failed");
        assert_eq!(row.tone, "attention");
        assert!(row.result.starts_with("It took too long"));
        assert_eq!(row.duration, "42 s");
        // A run that is still going has no result or duration yet.
        let running = RunRecord {
            status: "running".into(),
            summary: json!({"mode": "debug"}),
            ..run
        };
        let row = run_row(&running, "mode");
        assert_eq!(
            (row.what.as_str(), row.state_text.as_str()),
            ("debug", "Running")
        );
        assert_eq!(row.duration, "");
    }

    #[test]
    fn stored_times_are_shown_as_minutes() {
        let text = when_text("2026-11-22T10:15:30.123Z");
        assert_eq!(text.len(), "2026-11-22 10:15".len());
        assert!(text.starts_with("2026-11-2"));
    }
}
