//! Run text shared by the run tab and the Workflows run history: stored raw errors worded for
//! people, local times and the state of a run. (Moved from the Automations history when
//! Workflows replaced it.)

use senvakara_app::{CoreError, ErrorCode};

use crate::inventory::friendly_with;

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn stored_times_are_shown_as_minutes() {
        let text = when_text("2026-11-22T10:15:30.123Z");
        assert_eq!(text.len(), "2026-11-22 10:15".len());
        assert!(text.starts_with("2026-11-2"));
    }

    #[test]
    fn run_states_have_a_tone_and_a_word() {
        assert_eq!(run_state("succeeded"), ("live", "Succeeded"));
        assert_eq!(run_state("running"), ("busy", "Running"));
        assert_eq!(run_state("cancelled"), ("idle", "Cancelled"));
        assert_eq!(run_state("failed"), ("attention", "Failed"));
    }
}
