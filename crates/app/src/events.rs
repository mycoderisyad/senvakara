//! Events. Every session stream carries its own end event, so a viewer never
//! has to match a global event to its session. The broadcast [`AppEvent`]s
//! are refresh hints only: a receiver that lagged just reloads its lists.

/// Output of one terminal session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    Data(Vec<u8>),
    /// The shell ended (exit status if the server sent one). Last event.
    Exit(Option<u32>),
}

/// Output of one screen-mirroring session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorEvent {
    /// Codec string (e.g. `avc1.42c02a`), sent when the stream (re)configures.
    Config(String),
    /// One Annex-B access unit; the config is prepended on key frames.
    Packet { key: bool, pts: u64, data: Vec<u8> },
    /// The session ended on its own, with the reason. Last event.
    Ended(String),
}

/// Output of one logcat stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogcatEvent {
    Data(Vec<u8>),
    /// The stream ended on its own (device gone). Last event.
    Ended,
}

/// Refresh hints on [`crate::App::events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// The device list or a device's state changed.
    DevicesChanged,
    HostLost {
        host_id: String,
        reason: String,
    },
    HostReconnecting {
        host_id: String,
        attempt: u32,
    },
    HostReconnected {
        host_id: String,
        attempt: u32,
    },
}
