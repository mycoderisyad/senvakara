//! Structured error codes shared by adapters, the desktop core and the
//! (future) Linux host agent. Codes mirror docs/API.md §3.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Unauthenticated,
    Unauthorized,
    HostOffline,
    DeviceOffline,
    AdbUnauthorized,
    RouteStale,
    UnsupportedCapability,
    Timeout,
    Cancelled,
    Conflict,
    /// Request rejected by validation (bad address, unsafe file name, ...).
    InvalidInput,
    InternalError,
}

impl ErrorCode {
    /// Whether a bounded automatic retry is acceptable for this error.
    ///
    /// Only transport-level, side-effect-free failures qualify. Auth and
    /// routing failures must go back to the user (docs/API.md §10).
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorCode::HostOffline | ErrorCode::Timeout)
    }
}

/// Error returned across the core boundary. `message` must already be
/// redacted: never put secrets, private key paths or raw credentials here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("{code:?}: {message}")]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_as_api_contract_strings() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::RouteStale).unwrap(),
            "\"ROUTE_STALE\""
        );
        assert_eq!(
            serde_json::to_string(&ErrorCode::AdbUnauthorized).unwrap(),
            "\"ADB_UNAUTHORIZED\""
        );
    }

    #[test]
    fn only_transport_errors_are_retryable() {
        assert!(ErrorCode::Timeout.is_retryable());
        assert!(ErrorCode::HostOffline.is_retryable());
        assert!(!ErrorCode::RouteStale.is_retryable());
        assert!(!ErrorCode::Unauthorized.is_retryable());
        assert!(!ErrorCode::AdbUnauthorized.is_retryable());
    }
}
