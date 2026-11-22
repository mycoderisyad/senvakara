//! Workspaces: saved layouts of up to four panels, opened as a session tab whose panels are
//! live sessions of the existing kinds.
//!
//! The stored JSON is the v0.1.0 format (`WorkspacesView.tsx`), so workspaces saved by either
//! build open in the other. The only addition is `layout.slots`, which v0.1.0 ignores.

mod availability;
mod init;
mod labels;
mod naming;
mod pane;
mod panels;
mod present;
mod saved;
mod stored;
mod sync;
mod tab;

pub use pane::WorkspacePane;
pub use saved::State;

use serde_json::Value;

type DeviceView = senvakara_app::android::DeviceView;

// ------------------------------------------------------------------ model

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Single,
    Columns,
    Grid,
}

impl Layout {
    /// How many panels the layout shows.
    pub fn slots(self) -> usize {
        match self {
            Layout::Single => 1,
            Layout::Columns => 2,
            Layout::Grid => 4,
        }
    }

    /// The name stored in the database and used by the view.
    pub fn key(self) -> &'static str {
        match self {
            Layout::Single => "single",
            Layout::Columns => "columns",
            Layout::Grid => "grid",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Single => "1 panel",
            Layout::Columns => "2 columns",
            Layout::Grid => "2 x 2",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "single" => Some(Layout::Single),
            "columns" => Some(Layout::Columns),
            "grid" => Some(Layout::Grid),
            _ => None,
        }
    }

    /// From the stored layout object; anything missing or unknown is two columns, as v0.1.0.
    pub fn parse(layout: &Value) -> Self {
        layout
            .get("layout")
            .and_then(Value::as_str)
            .and_then(Self::from_key)
            .unwrap_or(Layout::Columns)
    }
}

/// The shell of a terminal panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shell {
    /// PowerShell on this PC.
    Windows,
    Wsl(Option<String>),
    /// `tmux` names the session to attach to, so reopening the workspace reattaches.
    Ssh {
        host_id: String,
        tmux: Option<String>,
    },
}

/// What a panel shows, as a reference that is checked again every time it is opened.
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    Terminal {
        title: String,
        shell: Shell,
    },
    Mirror {
        route_id: String,
    },
    Logcat {
        route_id: String,
    },
    Desktop {
        host_id: String,
    },
    Files {
        host_id: String,
    },
    /// A stored panel this build cannot read; kept as it was so saving does not destroy it.
    Unavailable {
        panel_type: String,
        target: Value,
    },
}
