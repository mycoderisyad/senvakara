//! The graph a user draws on the Workflows canvas, as stored in
//! `workflows.graph_json`. Positions are world units; the canvas scales them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version of the stored graph format. Bump it with a migration when a
/// stored field changes meaning.
pub const GRAPH_VERSION: u32 = 1;
pub const MAX_NODES: usize = 500;
pub const MAX_EDGES: usize = 2000;
pub const MAX_NODE_ID_LEN: usize = 64;
pub const MAX_TITLE_LEN: usize = 120;

fn graph_version() -> u32 {
    GRAPH_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default = "graph_version")]
    pub version: u32,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub viewport: Viewport,
}

impl Default for Workflow {
    fn default() -> Self {
        Self {
            version: GRAPH_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
            permissions: Permissions::default(),
            viewport: Viewport::default(),
        }
    }
}

impl Workflow {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

/// One node: an instance of a catalog module with its own settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    /// Catalog module id, for example `host.ssh` or `android.install_apk`.
    pub module: String,
    /// Title shown on the card; empty means the module name.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    /// Which real machine or phone an anchor node stands for. Only anchor
    /// modules (hosts, this PC, Android devices) have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
    /// Script files the user edited, by file name (`apply.sh`). The bundled
    /// file is used for every file not listed here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scripts: BTreeMap<String, String>,
}

/// The machine an anchor node points at. Ids refer to the Hosts and Devices
/// features; the graph never copies addresses or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    /// A host registered in Hosts, reached over SSH.
    Host { host_id: String },
    /// The PC Senvakara runs on.
    LocalPc,
    /// An Android device through one concrete route (ADR-018).
    Device { route_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// `from` (an action) runs on `to` (an anchor).
    Target,
    /// `from` runs before `to`.
    After,
    /// `to` uses the service `from` provides: `from` runs first, and its
    /// health shows on `to`.
    DependsOn,
}

impl EdgeKind {
    /// Whether the edge orders two actions.
    pub fn orders(self) -> bool {
        matches!(self, Self::After | Self::DependsOn)
    }
}

/// What a workflow may do, granted explicitly by the user (SECURITY §6,
/// generalised from the recipe permissions of FLOW-04).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Run module scripts as root through `sudo`.
    #[serde(default)]
    pub root: bool,
    /// Run commands the user typed (custom scripts, device shell).
    #[serde(default)]
    pub shell: bool,
    /// Install or remove apps and packages.
    #[serde(default)]
    pub install: bool,
    /// Change firewall, SSH or network settings.
    #[serde(default)]
    pub network: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default = "one")]
    pub zoom: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}
