//! Module manifests (`module.toml`): what a node does, what it needs and
//! which parameters it takes. The format is Senvakara's own (plan WF §2).

use serde::Deserialize;
use std::collections::BTreeMap;

pub const MAX_SCRIPT_LEN: usize = 64 * 1024;
const DEFAULT_TIMEOUT_SECS: u32 = 600;
const MAX_TIMEOUT_SECS: u32 = 3 * 60 * 60;

/// Script files a module may carry, in the order a run uses them.
pub const SCRIPT_FILES: [&str; 4] = ["check.sh", "apply.sh", "health.sh", "remove.sh"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub summary: String,
    /// Bumped whenever a bundled script changes, so a run records which
    /// revision it used.
    pub version: u32,
    pub executor: Executor,
    /// For an anchor: the kind of machine it stands for.
    #[serde(default)]
    pub binds: Option<BindKind>,
    /// Supported systems as `<os-release ID>>=<VERSION_ID>`, for example
    /// `debian>=12` (ADR-029). Empty means no restriction.
    #[serde(default)]
    pub os: Vec<String>,
    #[serde(default)]
    pub needs: Vec<Permission>,
    /// A bool parameter that, when true, makes the node need root (used by
    /// the custom script node).
    #[serde(default)]
    pub root_switch: Option<String>,
    /// The scripts come from the node itself, not from bundled files.
    #[serde(default)]
    pub user_scripts: bool,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    /// Vendor documentation the steps follow.
    #[serde(default)]
    pub docs_url: Option<String>,
}

fn default_timeout() -> u32 {
    DEFAULT_TIMEOUT_SECS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Host,
    Security,
    Networking,
    Runtime,
    Database,
    App,
    Deployment,
    Integration,
    Monitoring,
    Automation,
    Android,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "Host",
            Self::Security => "Security",
            Self::Networking => "Networking",
            Self::Runtime => "Runtime",
            Self::Database => "Database",
            Self::App => "App",
            Self::Deployment => "Deployment",
            Self::Integration => "Integration",
            Self::Monitoring => "Monitoring",
            Self::Automation => "Automation",
            Self::Android => "Android",
        }
    }
}

/// How a node is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Executor {
    /// Stands for a machine; runs nothing itself.
    Anchor,
    /// Bash scripts on a Linux host over SSH.
    SshScript,
    /// ADB commands on an Android device through its route.
    Adb,
    /// Flutter build on this PC, install on an Android device.
    LocalFlutter,
    /// Done by Senvakara itself (waiting).
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindKind {
    Host,
    LocalPc,
    Device,
}

impl BindKind {
    pub fn of(binding: &crate::graph::Binding) -> Self {
        match binding {
            crate::graph::Binding::Host { .. } => Self::Host,
            crate::graph::Binding::LocalPc => Self::LocalPc,
            crate::graph::Binding::Device { .. } => Self::Device,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "an SSH host",
            Self::LocalPc => "this PC",
            Self::Device => "an Android device",
        }
    }
}

impl Executor {
    /// The anchors a node with this executor may run on.
    pub fn runs_on(self) -> &'static [BindKind] {
        match self {
            Self::Anchor => &[],
            Self::SshScript => &[BindKind::Host],
            Self::Adb | Self::LocalFlutter => &[BindKind::Device],
            Self::Builtin => &[BindKind::Host, BindKind::LocalPc, BindKind::Device],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Root,
    Shell,
    Install,
    Network,
}

impl Permission {
    /// The switch the user turns on, as named in the workflow settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::Root => "run as root (sudo)",
            Self::Shell => "run shell commands",
            Self::Install => "install / uninstall",
            Self::Network => "change network and SSH settings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// Lowercase identifier; scripts read it as `SV_<NAME>`.
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
    /// Allowed values of a `choice`.
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub max_len: Option<usize>,
    /// Required ending of a `path`, compared case-insensitively (`.apk`).
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// One line of text.
    Text,
    /// Multi-line text such as a command list.
    Multiline,
    Int,
    Bool,
    Choice,
    Port,
    /// A path on the machine that reads it.
    Path,
    /// An Android application id.
    Package,
    /// A Linux user name.
    UnixUser,
    Hostname,
    /// A value kept in Credential Manager; the graph holds its reference.
    Secret,
}

/// A manifest with the files that ship next to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub manifest: Manifest,
    /// Catalog folder, `<category>/<id>`.
    pub path: String,
    pub files: BTreeMap<String, String>,
}

impl Module {
    pub fn is_anchor(&self) -> bool {
        self.manifest.executor == Executor::Anchor
    }

    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.manifest.params.iter().find(|p| p.name == name)
    }
}

pub(crate) fn valid_ident(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 48
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Structural checks a manifest must pass before the catalog accepts it.
pub(crate) fn check_manifest(m: &Manifest, files: &BTreeMap<String, String>) -> Result<(), String> {
    let id_ok = m.id.split('.').count() == 2 && m.id.split('.').all(valid_ident);
    if !id_ok {
        return Err(format!(
            "module id `{}` must look like `category.name`",
            m.id
        ));
    }
    if m.name.trim().is_empty() || m.summary.trim().is_empty() {
        return Err(format!("module {}: name and summary are required", m.id));
    }
    if (m.executor == Executor::Anchor) != m.binds.is_some() {
        return Err(format!(
            "module {}: anchors, and only anchors, set `binds`",
            m.id
        ));
    }
    if !(1..=MAX_TIMEOUT_SECS).contains(&m.timeout_secs) {
        return Err(format!("module {}: timeout_secs out of range", m.id));
    }
    for os in &m.os {
        if parse_os(os).is_none() {
            return Err(format!("module {}: bad os entry `{os}`", m.id));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in &m.params {
        if !valid_ident(&p.name) || !seen.insert(p.name.as_str()) {
            return Err(format!(
                "module {}: bad or repeated parameter `{}`",
                m.id, p.name
            ));
        }
        if p.kind == ParamKind::Choice && p.options.is_empty() {
            return Err(format!(
                "module {}: choice `{}` has no options",
                m.id, p.name
            ));
        }
    }
    if let Some(switch) = &m.root_switch {
        let is_bool = m
            .params
            .iter()
            .any(|p| &p.name == switch && p.kind == ParamKind::Bool);
        if !is_bool {
            return Err(format!(
                "module {}: root_switch must name a bool parameter",
                m.id
            ));
        }
    }
    for name in files.keys() {
        if name.ends_with(".sh") && !SCRIPT_FILES.contains(&name.as_str()) {
            return Err(format!("module {}: unexpected script {name}", m.id));
        }
    }
    let has_apply = files.contains_key("apply.sh");
    match m.executor {
        Executor::SshScript if !m.user_scripts && !has_apply => Err(format!(
            "module {}: an ssh_script module needs apply.sh",
            m.id
        )),
        Executor::SshScript => Ok(()),
        _ if files.keys().any(|f| f.ends_with(".sh")) => Err(format!(
            "module {}: only ssh_script modules carry scripts",
            m.id
        )),
        _ => Ok(()),
    }
}

/// `debian>=12` → (`debian`, 12.0). Versions compare as numbers, so
/// `ubuntu>=22.04` matches 22.04 and 24.04.
pub fn parse_os(entry: &str) -> Option<(&str, f64)> {
    let (id, version) = entry.split_once(">=")?;
    let id = id.trim();
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let version: f64 = version.trim().parse().ok()?;
    ok.then_some((id, version))
}

impl Manifest {
    /// Whether a host with this `/etc/os-release` `ID` and `VERSION_ID` may
    /// run the module.
    pub fn supports_os(&self, id: &str, version_id: &str) -> bool {
        if self.os.is_empty() {
            return true;
        }
        let version: Option<f64> = version_id.trim().parse().ok();
        self.os
            .iter()
            .filter_map(|e| parse_os(e))
            .any(|(want, min)| {
                want == id.trim().to_ascii_lowercase() && version.is_some_and(|v| v >= min)
            })
    }
}
