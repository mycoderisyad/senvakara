//! Text shared across the app: backend errors in plain words, action and fact rows, labels.

use senvakara_app::{CoreError, ErrorCode};
use slint::SharedString;

use crate::{Action, KeyValue};

/// A backend error in words a user can act on. The raw code and message stay in the log.
pub fn friendly(error: &CoreError) -> String {
    friendly_with(error, "A file with that name already exists.")
}

/// Like [`friendly`], for callers where a "conflict" is not about a file name: `conflict` says
/// what it means there.
pub fn friendly_with(error: &CoreError, conflict: &str) -> String {
    let lead = match error.code {
        ErrorCode::HostOffline => "The host did not answer. Check that it is on and reachable on this network.",
        ErrorCode::Unauthenticated => "The host refused the key. Check the username, the private key and its passphrase.",
        ErrorCode::Unauthorized => "This is not allowed.",
        ErrorCode::DeviceOffline => "The phone is not connected any more.",
        ErrorCode::AdbUnauthorized => "The phone has not allowed USB debugging from this computer. Unlock it and accept the prompt.",
        ErrorCode::RouteStale => "The phone was reconnected since this view was opened. Try again.",
        ErrorCode::UnsupportedCapability => "This is not available on that target.",
        ErrorCode::Timeout => "It took too long and was stopped.",
        ErrorCode::Cancelled => "Cancelled.",
        ErrorCode::Conflict => conflict,
        ErrorCode::InvalidInput => "Something in the request is not valid.",
        _ => "Something went wrong.",
    };
    if error.message.is_empty() {
        lead.to_string()
    } else {
        format!("{lead} ({})", error.message)
    }
}

pub fn action(id: &str, label: &str) -> Action {
    Action {
        id: id.into(),
        label: label.into(),
        reason: SharedString::new(),
        danger: false,
    }
}

pub fn blocked(id: &str, label: &str, reason: &str) -> Action {
    Action {
        reason: reason.into(),
        ..action(id, label)
    }
}

pub fn fact(key: &str, value: impl Into<SharedString>, mono: bool) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: value.into(),
        mono,
    }
}

/// "USB" or "Wi-Fi", from a route's connection type.
pub fn transport_label(connection_type: &str) -> &'static str {
    if connection_type.contains("wifi") || connection_type.contains("tcp") {
        "Wi-Fi"
    } else {
        "USB"
    }
}
