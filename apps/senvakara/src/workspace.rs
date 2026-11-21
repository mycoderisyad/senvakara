//! Workspaces: saved layouts of up to four panels, opened as a session tab whose panels are
//! live sessions of the existing kinds.
//!
//! The stored JSON is the v0.1.0 format (`WorkspacesView.tsx`), so workspaces saved by either
//! build open in the other. The only addition is `layout.slots`, which v0.1.0 ignores.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_adapters_pty::ShellKind;
use senvakara_app::{HostView, PanelRecord, WorkspaceRecord, WorkspaceSave};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::inventory::{desktop_problem, fact, friendly, transport_label};
use crate::{App, Pane, Pending, Place, Question, SlotView, TabInfo, WorkspaceItem, Workspaces};

type DeviceView = senvakara_app::android::DeviceView;

/// The panel types the database accepts.
const PANEL_TYPES: [&str; 5] = ["terminal", "mirror", "desktop", "files", "logcat"];
const NAME_LIMIT: usize = 80;

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

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn parse_shell(shell: &Value) -> Option<Shell> {
    match shell.get("kind").and_then(Value::as_str)? {
        "power_shell" => Some(Shell::Windows),
        "wsl" => Some(Shell::Wsl(text(shell, "distro"))),
        "ssh" => Some(Shell::Ssh {
            host_id: text(shell, "host_id")?,
            tmux: text(shell, "tmux"),
        }),
        _ => None,
    }
}

/// Read one stored panel. Anything unknown or broken becomes [`Spec::Unavailable`].
pub fn parse_panel(record: &PanelRecord) -> Spec {
    let target = &record.target;
    let parsed = match record.panel_type.as_str() {
        "terminal" => target.get("shell").and_then(parse_shell).map(|shell| {
            let title = text(target, "title").unwrap_or_else(|| match &shell {
                Shell::Windows => "PowerShell".into(),
                Shell::Wsl(Some(distro)) => format!("WSL {distro}"),
                Shell::Wsl(None) => "WSL".into(),
                Shell::Ssh { tmux: Some(_), .. } => "tmux".into(),
                Shell::Ssh { .. } => "SSH".into(),
            });
            Spec::Terminal { title, shell }
        }),
        "mirror" => text(target, "route_id").map(|route_id| Spec::Mirror { route_id }),
        "logcat" => text(target, "route_id").map(|route_id| Spec::Logcat { route_id }),
        "desktop" => text(target, "host_id").map(|host_id| Spec::Desktop { host_id }),
        "files" => text(target, "host_id").map(|host_id| Spec::Files { host_id }),
        _ => None,
    };
    parsed.unwrap_or_else(|| Spec::Unavailable {
        panel_type: record.panel_type.clone(),
        target: target.clone(),
    })
}

/// The stored form of a panel: exactly what v0.1.0 wrote.
pub fn panel_record(spec: &Spec) -> PanelRecord {
    let (panel_type, target) = match spec {
        Spec::Terminal { title, shell } => {
            let shell = match shell {
                Shell::Windows => json!({ "kind": "power_shell" }),
                Shell::Wsl(distro) => json!({ "kind": "wsl", "distro": distro }),
                Shell::Ssh { host_id, tmux } => {
                    json!({ "kind": "ssh", "host_id": host_id, "tmux": tmux })
                }
            };
            ("terminal", json!({ "title": title, "shell": shell }))
        }
        Spec::Mirror { route_id } => ("mirror", json!({ "route_id": route_id })),
        Spec::Logcat { route_id } => ("logcat", json!({ "route_id": route_id })),
        Spec::Desktop { host_id } => ("desktop", json!({ "host_id": host_id })),
        Spec::Files { host_id } => ("files", json!({ "host_id": host_id })),
        Spec::Unavailable { panel_type, target } => (panel_type.as_str(), target.clone()),
    };
    PanelRecord {
        panel_type: panel_type.to_string(),
        target,
    }
}

/// Which slot each stored panel goes to: `result[slot]` is the index of its panel.
///
/// `layout.slots` (one slot index per panel) is honoured where it is valid; panels without a
/// valid, free slot fill the remaining slots in order, which is all v0.1.0 did. Panels that do
/// not fit the layout are left out.
pub fn place_panels(count: usize, layout: &Value, capacity: usize) -> Vec<Option<usize>> {
    let mut slots: Vec<Option<usize>> = vec![None; capacity];
    let wanted: Vec<Option<usize>> = (0..count)
        .map(|panel| {
            layout
                .get("slots")
                .and_then(Value::as_array)
                .and_then(|list| list.get(panel))
                .and_then(Value::as_u64)
                .map(|slot| slot as usize)
                .filter(|slot| *slot < capacity)
        })
        .collect();
    let mut loose = Vec::new();
    for (panel, slot) in wanted.into_iter().enumerate() {
        match slot {
            Some(slot) if slots[slot].is_none() => slots[slot] = Some(panel),
            _ => loose.push(panel),
        }
    }
    let mut loose = loose.into_iter();
    for slot in slots.iter_mut().filter(|slot| slot.is_none()) {
        *slot = loose.next();
    }
    slots
}

/// A stored workspace as a layout and one optional panel per slot.
pub fn read_workspace(record: &WorkspaceRecord) -> (Layout, Vec<Option<Spec>>) {
    let layout = Layout::parse(&record.layout);
    let slots = place_panels(record.panels.len(), &record.layout, layout.slots())
        .into_iter()
        .map(|panel| panel.map(|index| parse_panel(&record.panels[index])))
        .collect();
    (layout, slots)
}

/// The layout object and the ordered panels to store. Panels of a type the database would
/// refuse (they cannot have come from it) are left out.
pub fn write_workspace(layout: Layout, slots: &[Option<Spec>]) -> (Value, Vec<PanelRecord>) {
    let mut positions = Vec::new();
    let mut panels = Vec::new();
    for (slot, spec) in slots.iter().enumerate().take(layout.slots()) {
        let Some(spec) = spec else { continue };
        let record = panel_record(spec);
        if PANEL_TYPES.contains(&record.panel_type.as_str()) {
            positions.push(slot);
            panels.push(record);
        }
    }
    (
        json!({ "layout": layout.key(), "slots": positions }),
        panels,
    )
}

/// Why the name cannot be saved, or "" when it can.
pub fn name_problem(name: &str) -> &'static str {
    let name = name.trim();
    if name.is_empty() {
        "Give the workspace a name to save it."
    } else if name.len() > NAME_LIMIT {
        "The name can be at most 80 characters."
    } else {
        ""
    }
}

/// `senvakara-<base36 time>`: the tmux session name v0.1.0 used.
pub fn tmux_name(millis: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut value = millis;
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    if out.is_empty() {
        out.push(b'0');
    }
    out.reverse();
    format!("senvakara-{}", String::from_utf8(out).unwrap_or_default())
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Why a panel cannot run right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocked {
    pub text: String,
    /// The host whose connection would fix it.
    pub connect: Option<String>,
}

fn route_text(device: &DeviceView) -> String {
    format!(
        "{} > {}",
        device.record.host_name,
        transport_label(&device.record.connection_type)
    )
}

fn trusted(host: &HostView) -> bool {
    host.trust_state == "trusted"
}

fn host_blocked(hosts: &[HostView], host_id: &str) -> Option<Blocked> {
    let Some(host) = hosts.iter().find(|host| host.id == host_id) else {
        return Some(Blocked {
            text: "This host was removed from Senvakara. Close the panel and add another.".into(),
            connect: None,
        });
    };
    if host.is_local || host.connected {
        return None;
    }
    if !trusted(host) {
        return Some(Blocked {
            text: format!(
                "The key of {} is not trusted any more. Verify it on Home, then connect.",
                host.display_name
            ),
            connect: None,
        });
    }
    Some(Blocked {
        text: format!("{} is not connected.", host.display_name),
        connect: Some(host.id.clone()),
    })
}

/// Why `spec` cannot run with the hosts and phones as they are, or `None` when it can.
pub fn blocked(spec: &Spec, hosts: &[HostView], devices: &[DeviceView]) -> Option<Blocked> {
    match spec {
        Spec::Terminal {
            shell: Shell::Ssh { host_id, .. },
            ..
        } => host_blocked(hosts, host_id),
        Spec::Terminal { .. } => None,
        Spec::Files { host_id } => host_blocked(hosts, host_id),
        Spec::Desktop { host_id } => host_blocked(hosts, host_id).or_else(|| {
            let host = hosts.iter().find(|host| host.id == *host_id)?;
            let desktop = host.capabilities.get("desktop");
            let session = desktop
                .and_then(|d| d.get("session"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            let has_x11vnc = desktop
                .and_then(|d| d.get("x11vnc"))
                .is_some_and(|v| v.as_str().is_some_and(|path| !path.is_empty()));
            let problem = desktop_problem(true, session, has_x11vnc);
            (!problem.is_empty()).then(|| Blocked {
                text: problem.to_string(),
                connect: None,
            })
        }),
        Spec::Mirror { route_id } | Spec::Logcat { route_id } => {
            let Some(device) = devices
                .iter()
                .find(|device| device.record.route_id == *route_id)
            else {
                return Some(Blocked {
                    text: "This phone was forgotten. Close the panel and add another.".into(),
                    connect: None,
                });
            };
            let record = &device.record;
            let name = format!("{} ({})", record.display_name, route_text(device));
            match record.state.as_str() {
                "available" => None,
                "unauthorized" => Some(Blocked {
                    text: format!(
                        "{name} has not allowed USB debugging from that computer. Unlock the phone and accept the prompt."
                    ),
                    connect: None,
                }),
                _ => Some(Blocked {
                    text: format!("{name} is not connected."),
                    connect: hosts
                        .iter()
                        .find(|host| host.id == record.host_id)
                        .filter(|host| !host.is_local && !host.connected && trusted(host))
                        .map(|host| host.id.clone()),
                }),
            }
        }
        Spec::Unavailable { panel_type, .. } => Some(Blocked {
            text: format!(
                "This saved panel ({panel_type}) could not be read. Close it and add another."
            ),
            connect: None,
        }),
    }
}

fn host_name(hosts: &[HostView], id: &str) -> String {
    hosts
        .iter()
        .find(|host| host.id == id)
        .map(|host| host.display_name.clone())
        .unwrap_or_else(|| "a removed host".into())
}

fn device_name(devices: &[DeviceView], route_id: &str) -> String {
    devices
        .iter()
        .find(|device| device.record.route_id == route_id)
        .map(|device| format!("{} ({})", device.record.display_name, route_text(device)))
        .unwrap_or_else(|| "a forgotten phone".into())
}

/// What the header of a panel says.
pub fn panel_title(spec: &Spec, hosts: &[HostView], devices: &[DeviceView]) -> String {
    match spec {
        Spec::Terminal { title, .. } => title.clone(),
        Spec::Desktop { host_id } => format!("Desktop of {}", host_name(hosts, host_id)),
        Spec::Files { host_id } => format!("Files on {}", host_name(hosts, host_id)),
        Spec::Mirror { route_id } => format!("Screen of {}", device_name(devices, route_id)),
        Spec::Logcat { route_id } => format!("Logcat of {}", device_name(devices, route_id)),
        Spec::Unavailable { panel_type, .. } => format!("Unavailable panel ({panel_type})"),
    }
}

/// The view that draws a panel, which is also its icon.
pub fn panel_kind(spec: &Spec) -> &'static str {
    match spec {
        Spec::Terminal { .. } | Spec::Unavailable { .. } => "terminal",
        Spec::Mirror { .. } => "mirror",
        Spec::Logcat { .. } => "logcat",
        Spec::Desktop { .. } => "desktop",
        Spec::Files { .. } => "files",
    }
}

/// Everything an empty panel can become, in the order of the picker. A tmux entry carries an
/// empty session name: the name is made when the panel is added.
pub fn panel_options(
    hosts: &[HostView],
    devices: &[DeviceView],
    distros: &[String],
) -> Vec<(String, Spec)> {
    let mut options = vec![(
        "Terminal: PowerShell".to_string(),
        Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows,
        },
    )];
    for distro in distros {
        options.push((
            format!("Terminal: WSL {distro}"),
            Spec::Terminal {
                title: format!("WSL {distro}"),
                shell: Shell::Wsl(Some(distro.clone())),
            },
        ));
    }
    for host in hosts.iter().filter(|host| !host.is_local && trusted(host)) {
        let name = &host.display_name;
        options.push((
            format!("Terminal: SSH to {name}"),
            Spec::Terminal {
                title: format!("SSH {name}"),
                shell: Shell::Ssh {
                    host_id: host.id.clone(),
                    tmux: None,
                },
            },
        ));
        if host
            .capabilities
            .get("tmux")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            options.push((
                format!("Terminal: tmux on {name} (survives disconnects)"),
                Spec::Terminal {
                    title: format!("tmux {name}"),
                    shell: Shell::Ssh {
                        host_id: host.id.clone(),
                        tmux: Some(String::new()),
                    },
                },
            ));
        }
        options.push((
            format!("Files: {name}"),
            Spec::Files {
                host_id: host.id.clone(),
            },
        ));
        options.push((
            format!("Desktop: {name}"),
            Spec::Desktop {
                host_id: host.id.clone(),
            },
        ));
    }
    for device in devices {
        let name = format!("{} ({})", device.record.display_name, route_text(device));
        let route_id = device.record.route_id.clone();
        options.push((
            format!("Screen: {name}"),
            Spec::Mirror {
                route_id: route_id.clone(),
            },
        ));
        options.push((format!("Logcat: {name}"), Spec::Logcat { route_id }));
    }
    options
}

/// One line naming the panels of a saved workspace.
pub fn summary(slots: &[Option<Spec>], hosts: &[HostView], devices: &[DeviceView]) -> String {
    let names: Vec<String> = slots
        .iter()
        .flatten()
        .map(|spec| panel_title(spec, hosts, devices))
        .collect();
    if names.is_empty() {
        "No panels".into()
    } else {
        names.join(", ")
    }
}

// ------------------------------------------------------------------- pane

/// One panel position of a workspace tab.
#[derive(Default)]
pub struct Slot {
    pub spec: Option<Spec>,
    /// The live session, while the panel can run.
    pub pane: Option<Pane>,
    /// Which entry of the picker is chosen while the slot is empty.
    choice: usize,
    /// A "Connect host" for this slot is in flight.
    connecting: bool,
}

/// Name, layout and one optional panel per slot, as last saved.
type Stored = (String, Layout, Vec<Option<Spec>>);

/// A workspace tab.
pub struct WorkspacePane {
    pub id: i32,
    /// The database id, once saved.
    saved_id: RefCell<Option<String>>,
    name: RefCell<String>,
    layout: Cell<Layout>,
    pub slots: RefCell<Vec<Slot>>,
    /// What the view draws, one row per slot.
    pub view: Rc<VecModel<SlotView>>,
    /// Name, layout and panels as last saved; `None` for a workspace never saved.
    stored: RefCell<Option<Stored>>,
    /// The picker entries, in the order shown.
    options: RefCell<Vec<(String, Spec)>>,
    /// (is good news, text) next to Save.
    message: RefCell<(bool, String)>,
}

impl WorkspacePane {
    fn new(id: i32, name: &str, layout: Layout, specs: Vec<Option<Spec>>) -> Self {
        let mut slots: Vec<Slot> = specs
            .into_iter()
            .map(|spec| Slot {
                spec,
                ..Slot::default()
            })
            .collect();
        slots.resize_with(layout.slots(), Slot::default);
        Self {
            id,
            saved_id: RefCell::new(None),
            name: RefCell::new(name.to_string()),
            layout: Cell::new(layout),
            slots: RefCell::new(slots),
            view: Rc::new(VecModel::default()),
            stored: RefCell::new(None),
            options: RefCell::new(Vec::new()),
            message: RefCell::new((true, String::new())),
        }
    }

    fn specs(&self) -> Vec<Option<Spec>> {
        self.slots
            .borrow()
            .iter()
            .map(|slot| slot.spec.clone())
            .collect()
    }

    /// Whether anything differs from what is stored.
    fn dirty(&self) -> bool {
        match &*self.stored.borrow() {
            Some((name, layout, specs)) => {
                *name != self.name.borrow().trim()
                    || *layout != self.layout.get()
                    || *specs != self.specs()
            }
            None => true,
        }
    }

    /// The slot holding the session `id`.
    pub fn slot_of(&self, id: i32) -> Option<usize> {
        self.slots
            .borrow()
            .iter()
            .position(|slot| slot.pane.as_ref().is_some_and(|pane| pane.id() == id))
    }

    /// Whether closing the tab would end a session that is still going.
    pub fn running(&self) -> bool {
        self.slots
            .borrow()
            .iter()
            .any(|slot| slot.pane.as_ref().is_some_and(Pane::running))
    }

    pub fn closing_note(&self) -> String {
        let mut note = "Every session in its panels will end.".to_string();
        if self.dirty() {
            note.push_str(" Changes that were not saved are lost.");
        }
        note
    }
}

/// The Workspaces section: what is saved.
#[derive(Default)]
pub struct State {
    records: RefCell<Vec<WorkspaceRecord>>,
    items: Rc<VecModel<WorkspaceItem>>,
}

impl State {
    /// (id, name) of every saved workspace, for the command palette.
    pub fn names(&self) -> Vec<(String, String)> {
        self.records
            .borrow()
            .iter()
            .map(|record| (record.id.clone(), record.name.clone()))
            .collect()
    }
}

impl App {
    pub fn init_workspaces(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Workspaces>();
        view.set_saved(self.ws.items.clone().into());
        view.on_open({
            let app = self.clone();
            move |id| app.workspace_open(&id)
        });
        view.on_create({
            let app = self.clone();
            move || app.workspace_create()
        });
        view.on_remove({
            let app = self.clone();
            move |id| app.workspace_remove_ask(&id)
        });
        view.on_name_edited({
            let app = self.clone();
            move |text| {
                if let Some(tab) = app.workspace_tab() {
                    app.with_workspace(tab, |pane| *pane.name.borrow_mut() = text.to_string());
                    app.workspace_publish(tab);
                }
            }
        });
        view.on_set_layout({
            let app = self.clone();
            move |key| {
                if let (Some(tab), Some(layout)) = (app.workspace_tab(), Layout::from_key(&key)) {
                    app.workspace_layout_ask(tab, layout);
                }
            }
        });
        view.on_save({
            let app = self.clone();
            move || app.workspace_save()
        });
        view.on_choose({
            let app = self.clone();
            move |slot, index| {
                if let Some(tab) = app.workspace_tab() {
                    app.with_workspace(tab, |pane| {
                        if let Some(slot) = pane.slots.borrow_mut().get_mut(slot.max(0) as usize) {
                            slot.choice = index.max(0) as usize;
                        }
                    });
                    app.workspace_publish(tab);
                }
            }
        });
        view.on_add({
            let app = self.clone();
            move |slot| app.workspace_add(slot.max(0) as usize)
        });
        view.on_close_panel({
            let app = self.clone();
            move |slot| app.workspace_close_ask(slot.max(0) as usize)
        });
        view.on_connect({
            let app = self.clone();
            move |slot| app.workspace_connect(slot.max(0) as usize)
        });
        self.workspaces_reload();
    }

    /// Read the saved workspaces again and redraw the section.
    pub fn workspaces_reload(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Workspaces>();
        match self.backend.workspaces_list() {
            Ok(records) => {
                *self.ws.records.borrow_mut() = records;
                view.set_error(SharedString::new());
            }
            Err(error) => view.set_error(friendly(&error).into()),
        }
        self.workspaces_list_publish();
    }

    fn workspaces_list_publish(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let open: Vec<String> = self
            .sessions
            .borrow()
            .iter()
            .filter_map(|pane| match pane {
                Pane::Workspace(pane) => pane.saved_id.borrow().clone(),
                _ => None,
            })
            .collect();
        let items: Vec<WorkspaceItem> = self
            .ws
            .records
            .borrow()
            .iter()
            .map(|record| {
                let (layout, slots) = read_workspace(record);
                WorkspaceItem {
                    id: record.id.as_str().into(),
                    name: record.name.as_str().into(),
                    layout_text: layout.label().into(),
                    summary: summary(&slots, &hosts, &devices).into(),
                    open: open.contains(&record.id),
                }
            })
            .collect();
        let current: Vec<WorkspaceItem> = self.ws.items.iter().collect();
        if current != items {
            ui.global::<crate::Nav>()
                .set_workspace_count(items.len().to_string().into());
            self.ws.items.set_vec(items);
        }
    }

    /// The index of the workspace tab on screen.
    fn workspace_tab(&self) -> Option<usize> {
        let index = self.active.get();
        let sessions = self.sessions.borrow();
        match (index >= 0).then(|| sessions.get(index as usize)).flatten() {
            Some(Pane::Workspace(_)) => Some(index as usize),
            _ => None,
        }
    }

    /// The tab that holds the workspace pane `id`.
    fn workspace_tab_of(&self, id: i32) -> Option<usize> {
        self.sessions
            .borrow()
            .iter()
            .position(|pane| matches!(pane, Pane::Workspace(pane) if pane.id == id))
    }

    fn with_workspace<R>(&self, tab: usize, f: impl FnOnce(&WorkspacePane) -> R) -> Option<R> {
        match self.sessions.borrow().get(tab) {
            Some(Pane::Workspace(pane)) => Some(f(pane)),
            _ => None,
        }
    }

    fn workspace_add_tab(self: &Rc<Self>, pane: WorkspacePane) {
        let (id, title) = (pane.id, pane.name.borrow().clone());
        self.sessions.borrow_mut().push(Pane::Workspace(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "workspace".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue: 0,
            exited: false,
        });
        let tab = self.tabs.row_count() - 1;
        self.workspace_sync(tab);
        self.select(tab as i32);
        self.workspaces_list_publish();
    }

    /// "New workspace": an unsaved tab with two empty panels.
    pub fn workspace_create(self: &Rc<Self>) {
        let id = self.take_id();
        self.workspace_add_tab(WorkspacePane::new(
            id,
            "New workspace",
            Layout::Columns,
            Vec::new(),
        ));
    }

    /// Open a saved workspace in a tab and start its sessions; show its tab if it is open.
    pub fn workspace_open(self: &Rc<Self>, saved_id: &str) {
        let open = self.sessions.borrow().iter().position(|pane| {
            matches!(pane, Pane::Workspace(pane) if pane.saved_id.borrow().as_deref() == Some(saved_id))
        });
        if let Some(tab) = open {
            self.select(tab as i32);
            return;
        }
        let record = self
            .ws
            .records
            .borrow()
            .iter()
            .find(|record| record.id == saved_id)
            .cloned();
        let Some(record) = record else { return };
        let (layout, specs) = read_workspace(&record);
        let pane = WorkspacePane::new(self.take_id(), &record.name, layout, specs.clone());
        *pane.saved_id.borrow_mut() = Some(record.id.clone());
        *pane.stored.borrow_mut() = Some((record.name.trim().to_string(), layout, specs));
        self.workspace_add_tab(pane);
    }

    fn workspace_remove_ask(&self, saved_id: &str) {
        let record = self
            .ws
            .records
            .borrow()
            .iter()
            .find(|record| record.id == saved_id)
            .cloned();
        let Some(record) = record else { return };
        let (layout, slots) = read_workspace(&record);
        let facts = vec![
            fact("Workspace", record.name.as_str(), false),
            fact("Layout", layout.label(), false),
            fact(
                "Panels",
                summary(&slots, &self.inv.hosts(), &self.inv.devices()),
                false,
            ),
        ];
        self.ask_with(
            Question {
                title: format!("Delete {}?", record.name),
                body: "The saved layout is removed. If it is open, its tab is closed and its sessions end. Hosts and phones are not affected.".into(),
                confirm: "Delete workspace".into(),
                danger: true,
            },
            facts,
            "",
            Pending::DeleteWorkspace(record.id),
        );
    }

    /// The user confirmed deleting a saved workspace.
    pub fn workspace_delete(self: &Rc<Self>, saved_id: String) {
        let open = self.sessions.borrow().iter().position(|pane| {
            matches!(pane, Pane::Workspace(pane) if pane.saved_id.borrow().as_deref() == Some(saved_id.as_str()))
        });
        if let Err(error) = self.backend.workspace_delete(saved_id) {
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<Workspaces>().set_error(friendly(&error).into());
            }
            return;
        }
        if let Some(tab) = open {
            self.close(tab as i32);
        }
        self.workspaces_reload();
    }

    /// Hosts or phones changed: panels start or explain themselves, pickers follow.
    pub fn workspaces_inventory_changed(self: &Rc<Self>) {
        let tabs: Vec<usize> = self
            .sessions
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, pane)| matches!(pane, Pane::Workspace(_)))
            .map(|(tab, _)| tab)
            .collect();
        for tab in tabs {
            self.workspace_sync(tab);
        }
        self.workspaces_list_publish();
    }

    /// Bring the panels of a workspace in line with what can run: start the ones that can and
    /// have no session, and drop ended sessions whose target went away so the panel says why.
    fn workspace_sync(self: &Rc<Self>, tab: usize) {
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let plan = self.with_workspace(tab, |pane| {
            let mut start = Vec::new();
            let mut slots = pane.slots.borrow_mut();
            for (index, slot) in slots.iter_mut().enumerate() {
                let Some(spec) = &slot.spec else { continue };
                let reason = blocked(spec, &hosts, &devices);
                match (&slot.pane, reason) {
                    (None, None) => start.push((index, spec.clone())),
                    (Some(live), Some(_)) if !live.running() => slot.pane = None,
                    _ => {}
                }
            }
            start
        });
        for (index, spec) in plan.unwrap_or_default() {
            let Some(live) = self.start_panel(&spec) else {
                continue;
            };
            let id = live.id();
            let placed = self.with_workspace(tab, |pane| {
                let mut slots = pane.slots.borrow_mut();
                match slots.get_mut(index) {
                    Some(slot) if slot.spec.as_ref() == Some(&spec) && slot.pane.is_none() => {
                        slot.pane = Some(live);
                        true
                    }
                    _ => false,
                }
            });
            if placed == Some(true) {
                self.pane_started(id);
            }
        }
        self.workspace_publish(tab);
    }

    /// Start the session a panel describes. The caller has checked that it can run.
    fn start_panel(self: &Rc<Self>, spec: &Spec) -> Option<Pane> {
        let id = self.take_id();
        let (cols, lines) = self.grid.get();
        match spec {
            Spec::Terminal { shell, .. } => match shell {
                Shell::Windows | Shell::Wsl(_) => {
                    let kind = match shell {
                        Shell::Wsl(distro) => ShellKind::Wsl {
                            distro: distro.clone(),
                        },
                        _ => ShellKind::PowerShell,
                    };
                    match crate::term::Session::spawn(id, &kind, cols, lines, crate::notifier(id)) {
                        Ok(session) => Some(Pane::Term(session)),
                        Err(error) => {
                            // A terminal that says why it could not start, instead of nothing.
                            let (session, feeder, _commands) =
                                crate::term::Session::remote(id, cols, lines, crate::notifier(id));
                            feeder.feed(
                                format!("The shell could not be started: {error:#}\r\n").as_bytes(),
                            );
                            feeder.end();
                            Some(Pane::Term(session))
                        }
                    }
                }
                Shell::Ssh { host_id, tmux } => {
                    let host = self.inv.host(host_id)?;
                    Some(Pane::Term(self.start_ssh(id, &host, tmux.clone())))
                }
            },
            Spec::Mirror { route_id } | Spec::Logcat { route_id } => {
                let device = self.inv.device(route_id)?;
                let record = &device.record;
                let route = format!(
                    "{}  >  {}",
                    record.host_name,
                    transport_label(&record.connection_type)
                );
                let hue = self.inv.hue(&record.host_id);
                Some(if matches!(spec, Spec::Mirror { .. }) {
                    Pane::Mirror(crate::mirror::MirrorPane::start(
                        id,
                        self.backend.clone(),
                        record.route_id.clone(),
                        record.display_name.clone(),
                        route,
                        hue,
                        crate::notifier(id),
                    ))
                } else {
                    Pane::Logcat(crate::logcat::LogcatPane::start(
                        id,
                        self.backend.clone(),
                        record.route_id.clone(),
                        record.display_name.clone(),
                        route,
                        hue,
                        crate::notifier(id),
                    ))
                })
            }
            Spec::Desktop { host_id } => {
                let host = self.inv.host(host_id)?;
                Some(Pane::Desktop(crate::desktop::DesktopPane::start(
                    id,
                    self.backend.clone(),
                    host.id.clone(),
                    host.display_name.clone(),
                    self.inv.hue(&host.id),
                    crate::notifier(id),
                )))
            }
            Spec::Files { host_id } => Some(Pane::Files(crate::files::FilesPane::new(
                id,
                crate::files::Source::Host(host_id.clone()),
            ))),
            Spec::Unavailable { .. } => None,
        }
    }

    /// The hue of the machine a panel runs on.
    fn spec_hue(&self, spec: &Spec) -> i32 {
        match spec {
            Spec::Terminal {
                shell: Shell::Ssh { host_id, .. },
                ..
            }
            | Spec::Desktop { host_id }
            | Spec::Files { host_id } => self.inv.hue(host_id),
            Spec::Terminal {
                shell: Shell::Wsl(_),
                ..
            } => 4,
            Spec::Terminal { .. } => 1,
            Spec::Mirror { route_id } | Spec::Logcat { route_id } => self
                .inv
                .device(route_id)
                .map(|device| self.inv.hue(&device.record.host_id))
                .unwrap_or(0),
            Spec::Unavailable { .. } => 0,
        }
    }

    /// Redraw a workspace: its panel rows always (they are its own model), and the toolbar and
    /// the picker when it is the tab on screen.
    pub fn workspace_publish(self: &Rc<Self>, tab: usize) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let options = panel_options(&hosts, &devices, &self.inv.wsl());
        let visible = self.active.get() == tab as i32;
        let mut fresh = Vec::new();
        let title = self.with_workspace(tab, |pane| {
            let slots = pane.slots.borrow();
            let rows: Vec<SlotView> = slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    let old = pane.view.row_data(index).unwrap_or_default();
                    let Some(spec) = &slot.spec else {
                        return SlotView {
                            state: "empty".into(),
                            choice: slot.choice.min(options.len().saturating_sub(1)) as i32,
                            ..SlotView::default()
                        };
                    };
                    let mut row = SlotView {
                        title: panel_title(spec, &hosts, &devices).into(),
                        kind: panel_kind(spec).into(),
                        hue: self.spec_hue(spec),
                        ..SlotView::default()
                    };
                    match &slot.pane {
                        Some(live) => {
                            row.state = "live".into();
                            // Keep what the session has drawn unless it is another session.
                            if old.state == "live" && panel_id(&old.panel) == live.id() {
                                row.panel = old.panel;
                            } else {
                                fresh.push(index);
                            }
                        }
                        None => {
                            let reason = blocked(spec, &hosts, &devices);
                            row.state = "blocked".into();
                            row.can_connect = reason.as_ref().is_some_and(|r| r.connect.is_some());
                            row.connecting = slot.connecting;
                            row.reason = reason
                                .map(|r| r.text)
                                .unwrap_or_else(|| "Starting.".into())
                                .into();
                        }
                    }
                    row
                })
                .collect();
            if pane.view.row_count() != rows.len() {
                pane.view.set_vec(rows);
            } else {
                for (index, row) in rows.into_iter().enumerate() {
                    if pane.view.row_data(index).as_ref() != Some(&row) {
                        pane.view.set_row_data(index, row);
                    }
                }
            }
            let name = pane.name.borrow().trim().to_string();
            if visible {
                let view = ui.global::<Workspaces>();
                let dirty = pane.dirty();
                let reason = name_problem(&name);
                let saved = pane.saved_id.borrow().is_some();
                view.set_name(pane.name.borrow().as_str().into());
                view.set_layout(pane.layout.get().key().into());
                view.set_slots(pane.view.clone().into());
                let labels: Vec<SharedString> =
                    options.iter().map(|(label, _)| label.into()).collect();
                let current: Vec<SharedString> = view.get_options().iter().collect();
                if current != labels {
                    view.set_options(ModelRc::new(VecModel::from(labels)));
                }
                view.set_dirty(dirty);
                view.set_save_label(
                    if !saved {
                        "Save workspace"
                    } else if dirty {
                        "Save changes"
                    } else {
                        "Saved"
                    }
                    .into(),
                );
                view.set_save_reason(
                    if !reason.is_empty() {
                        reason
                    } else if !dirty {
                        "There are no changes to save."
                    } else {
                        ""
                    }
                    .into(),
                );
                let message = pane.message.borrow();
                // A "Saved" note is stale as soon as something changes again.
                let show = !(message.0 && dirty);
                view.set_message(if show { message.1.as_str() } else { "" }.into());
                view.set_message_ok(message.0);
            }
            *pane.options.borrow_mut() = options;
            if name.is_empty() {
                "Workspace".to_string()
            } else {
                name
            }
        });
        if let (Some(title), Some(mut info)) = (title, self.tabs.row_data(tab)) {
            if info.title != title.as_str() {
                info.title = title.into();
                self.tabs.set_row_data(tab, info);
            }
        }
        // Sessions that just got a row draw themselves into it.
        if visible {
            for slot in fresh {
                self.present_place(&ui, Place::Slot(tab, slot), true);
            }
        }
    }

    /// The workspace tab came on screen: show its toolbar and redraw every live panel.
    pub fn present_workspace(self: &Rc<Self>, ui: &crate::MainWindow, tab: usize) {
        self.workspace_publish(tab);
        let live: Vec<usize> = self
            .with_workspace(tab, |pane| {
                pane.slots
                    .borrow()
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.pane.is_some())
                    .map(|(index, _)| index)
                    .collect()
            })
            .unwrap_or_default();
        for slot in live {
            self.present_place(ui, Place::Slot(tab, slot), true);
        }
    }

    fn workspace_add(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        self.with_workspace(tab, |pane| {
            let mut slots = pane.slots.borrow_mut();
            let Some(slot) = slots.get_mut(index) else {
                return;
            };
            if slot.spec.is_some() {
                return;
            }
            let Some((_, spec)) = pane.options.borrow().get(slot.choice).cloned() else {
                return;
            };
            slot.spec = Some(match spec {
                Spec::Terminal {
                    title,
                    shell:
                        Shell::Ssh {
                            host_id,
                            tmux: Some(_),
                        },
                } => Spec::Terminal {
                    title,
                    shell: Shell::Ssh {
                        host_id,
                        tmux: Some(tmux_name(now_millis())),
                    },
                },
                other => other,
            });
            slot.choice = 0;
        });
        self.workspace_sync(tab);
    }

    fn workspace_close_ask(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let found = self
            .with_workspace(tab, |pane| {
                let slots = pane.slots.borrow();
                let slot = slots.get(index)?;
                let running = slot.pane.as_ref().is_some_and(Pane::running);
                let note = slot
                    .pane
                    .as_ref()
                    .map(Pane::closing_note)
                    .unwrap_or_default();
                Some((pane.id, running, note))
            })
            .flatten();
        let Some((id, running, note)) = found else {
            return;
        };
        if !running {
            self.workspace_clear(id, index);
            return;
        }
        let title = self
            .with_workspace(tab, |pane| pane.view.row_data(index))
            .flatten()
            .map(|row| row.title.to_string())
            .unwrap_or_default();
        self.ask(
            Question {
                title: format!("Close {title}?"),
                body: format!("{note} The panel becomes empty; save the workspace to keep that."),
                confirm: "Close panel".into(),
                danger: true,
            },
            Pending::CloseWorkspacePanel {
                workspace: id,
                slot: index,
            },
        );
    }

    /// Empty a panel of the workspace pane `id`, ending its session.
    pub fn workspace_clear(self: &Rc<Self>, id: i32, index: usize) {
        let Some(tab) = self.workspace_tab_of(id) else {
            return;
        };
        let old = self
            .with_workspace(tab, |pane| {
                pane.slots.borrow_mut().get_mut(index).map(std::mem::take)
            })
            .flatten();
        // Dropped after the borrow ends: ending a session may ask for a redraw.
        drop(old);
        self.workspace_publish(tab);
    }

    fn workspace_layout_ask(self: &Rc<Self>, tab: usize, layout: Layout) {
        let found = self.with_workspace(tab, |pane| {
            let lost: Vec<usize> = pane
                .slots
                .borrow()
                .iter()
                .enumerate()
                .skip(layout.slots())
                .filter(|(_, slot)| slot.spec.is_some())
                .map(|(index, _)| index)
                .collect();
            (pane.id, pane.layout.get(), lost)
        });
        let Some((id, current, lost)) = found else {
            return;
        };
        if current == layout {
            return;
        }
        if lost.is_empty() {
            self.workspace_set_layout(id, layout);
            return;
        }
        let titles: Vec<String> = self
            .with_workspace(tab, |pane| {
                lost.iter()
                    .filter_map(|index| pane.view.row_data(*index))
                    .map(|row| row.title.to_string())
                    .collect()
            })
            .unwrap_or_default();
        self.ask_with(
            Question {
                title: format!("Switch to {}?", layout.label()),
                body: "The panels that do not fit are closed and their sessions end.".into(),
                confirm: "Switch layout".into(),
                danger: true,
            },
            vec![fact("Closed", titles.join(", "), false)],
            "",
            Pending::WorkspaceLayout {
                workspace: id,
                layout,
            },
        );
    }

    pub fn workspace_set_layout(self: &Rc<Self>, id: i32, layout: Layout) {
        let Some(tab) = self.workspace_tab_of(id) else {
            return;
        };
        let dropped = self.with_workspace(tab, |pane| {
            pane.layout.set(layout);
            let mut slots = pane.slots.borrow_mut();
            let rest: Vec<Slot> = if slots.len() > layout.slots() {
                slots.drain(layout.slots()..).collect()
            } else {
                Vec::new()
            };
            slots.resize_with(layout.slots(), Slot::default);
            rest
        });
        drop(dropped);
        self.workspace_sync(tab);
    }

    fn workspace_connect(self: &Rc<Self>, index: usize) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let (hosts, devices) = (self.inv.hosts(), self.inv.devices());
        let found = self
            .with_workspace(tab, |pane| {
                let mut slots = pane.slots.borrow_mut();
                let slot = slots.get_mut(index)?;
                let host = blocked(slot.spec.as_ref()?, &hosts, &devices)?.connect?;
                slot.connecting = true;
                Some((pane.id, host))
            })
            .flatten();
        let Some((id, host_id)) = found else { return };
        self.workspace_publish(tab);
        let backend = self.backend.clone();
        self.run(
            async move { backend.host_connect(host_id).await },
            move |app, result| {
                let Some(tab) = app.workspace_tab_of(id) else {
                    return;
                };
                app.with_workspace(tab, |pane| {
                    if let Some(slot) = pane.slots.borrow_mut().get_mut(index) {
                        slot.connecting = false;
                    }
                    *pane.message.borrow_mut() = match &result {
                        Ok(_) => (true, String::new()),
                        Err(error) => (false, friendly(error)),
                    };
                });
                // Reloading the inventory starts every panel that can run now.
                app.reload();
                app.workspace_publish(tab);
            },
        );
    }

    fn workspace_save(self: &Rc<Self>) {
        let Some(tab) = self.workspace_tab() else {
            return;
        };
        let input = self.with_workspace(tab, |pane| {
            let name = pane.name.borrow().trim().to_string();
            if !name_problem(&name).is_empty() {
                return None;
            }
            let (layout, panels) = write_workspace(pane.layout.get(), &pane.specs());
            Some(WorkspaceSave {
                id: pane.saved_id.borrow().clone(),
                name,
                layout,
                panels,
            })
        });
        let Some(Some(input)) = input else { return };
        let result = self.backend.workspace_save(input);
        self.with_workspace(tab, |pane| match &result {
            Ok(record) => {
                *pane.saved_id.borrow_mut() = Some(record.id.clone());
                *pane.stored.borrow_mut() = Some((
                    record.name.trim().to_string(),
                    pane.layout.get(),
                    pane.specs(),
                ));
                *pane.message.borrow_mut() = (true, format!("Saved {}.", record.name));
            }
            Err(error) => {
                *pane.message.borrow_mut() = (
                    false,
                    crate::inventory::friendly_with(
                        error,
                        "This workspace was deleted in the meantime.",
                    ),
                );
            }
        });
        self.workspace_publish(tab);
        self.workspaces_reload();
    }
}

/// The session a drawn panel belongs to.
fn panel_id(panel: &crate::PanelView) -> i32 {
    match panel.kind.as_str() {
        "terminal" => panel.term.id,
        "mirror" => panel.mirror.id,
        "logcat" => panel.logcat.id,
        "files" => panel.files.id,
        "desktop" => panel.desktop.id,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(panel_type: &str, target: Value) -> PanelRecord {
        PanelRecord {
            panel_type: panel_type.into(),
            target,
        }
    }

    fn record(layout: Value, panels: Vec<PanelRecord>) -> WorkspaceRecord {
        WorkspaceRecord {
            id: "ws_1".into(),
            name: "Bench".into(),
            layout,
            panels,
            updated_at: String::new(),
        }
    }

    /// What `WorkspacesView.tsx` of v0.1.0 stored for one panel of every kind.
    fn v010_panels() -> Vec<PanelRecord> {
        vec![
            panel(
                "terminal",
                json!({"title": "PowerShell", "shell": {"kind": "power_shell"}}),
            ),
            panel(
                "terminal",
                json!({"title": "WSL · Ubuntu", "shell": {"kind": "wsl", "distro": "Ubuntu"}}),
            ),
            panel(
                "terminal",
                json!({"title": "tmux · fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": "senvakara-m3x9k2"}}),
            ),
            panel("mirror", json!({"route_id": "route_1"})),
        ]
    }

    #[test]
    fn reads_what_v010_wrote() {
        let (layout, slots) = read_workspace(&record(json!({"layout": "grid"}), v010_panels()));
        assert_eq!(layout, Layout::Grid);
        assert_eq!(
            slots,
            vec![
                Some(Spec::Terminal {
                    title: "PowerShell".into(),
                    shell: Shell::Windows
                }),
                Some(Spec::Terminal {
                    title: "WSL · Ubuntu".into(),
                    shell: Shell::Wsl(Some("Ubuntu".into()))
                }),
                Some(Spec::Terminal {
                    title: "tmux · fedora".into(),
                    shell: Shell::Ssh {
                        host_id: "host_a".into(),
                        tmux: Some("senvakara-m3x9k2".into())
                    }
                }),
                Some(Spec::Mirror {
                    route_id: "route_1".into()
                }),
            ]
        );
    }

    #[test]
    fn every_panel_kind_round_trips_in_the_v010_shape() {
        let stored = vec![
            panel(
                "terminal",
                json!({"title": "SSH · fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": null}}),
            ),
            panel(
                "terminal",
                json!({"title": "WSL", "shell": {"kind": "wsl", "distro": null}}),
            ),
            panel("logcat", json!({"route_id": "route_1"})),
            panel("desktop", json!({"host_id": "host_a"})),
            panel("files", json!({"host_id": "host_a"})),
            panel("mirror", json!({"route_id": "route_2"})),
        ];
        for record in stored {
            assert_eq!(panel_record(&parse_panel(&record)), record);
        }
    }

    #[test]
    fn a_missing_or_unknown_layout_is_two_columns() {
        for layout in [
            json!({}),
            json!(null),
            json!({"layout": "ring"}),
            json!("x"),
        ] {
            let (parsed, slots) = read_workspace(&record(layout, v010_panels()));
            assert_eq!(parsed, Layout::Columns);
            // Only the first two panels fit, as in v0.1.0.
            assert_eq!(slots.len(), 2);
            assert!(matches!(slots[1], Some(Spec::Terminal { .. })));
        }
    }

    #[test]
    fn broken_panels_become_unavailable_and_are_kept_as_stored() {
        let broken = vec![
            panel("terminal", json!({"title": "x"})),
            panel("terminal", json!({"shell": {"kind": "telnet"}})),
            panel("terminal", json!({"shell": {"kind": "ssh"}})),
            panel("mirror", json!({})),
            panel("files", json!({"host_id": 7})),
            panel("desktop", json!(null)),
            panel("hologram", json!({"x": 1})),
        ];
        for record in &broken {
            let spec = parse_panel(record);
            assert!(matches!(spec, Spec::Unavailable { .. }), "{record:?}");
            assert_eq!(&panel_record(&spec), record);
            assert!(blocked(&spec, &[], &[]).is_some());
        }
        // A panel type the database refuses is not written back.
        let slots: Vec<Option<Spec>> = broken.iter().map(|r| Some(parse_panel(r))).collect();
        let (_, panels) = write_workspace(Layout::Grid, &slots[3..]);
        assert_eq!(panels.len(), 3);
        assert!(panels.iter().all(|p| p.panel_type != "hologram"));
    }

    #[test]
    fn a_terminal_without_a_title_gets_one() {
        let spec = parse_panel(&panel(
            "terminal",
            json!({"shell": {"kind": "power_shell"}}),
        ));
        assert_eq!(
            spec,
            Spec::Terminal {
                title: "PowerShell".into(),
                shell: Shell::Windows
            }
        );
    }

    #[test]
    fn panels_fill_slots_in_order_without_positions() {
        assert_eq!(
            place_panels(2, &json!({"layout": "grid"}), 4),
            vec![Some(0), Some(1), None, None]
        );
        assert_eq!(place_panels(3, &json!({}), 2), vec![Some(0), Some(1)]);
        assert_eq!(place_panels(0, &json!({}), 1), vec![None]);
    }

    #[test]
    fn stored_positions_put_panels_back_where_they_were() {
        assert_eq!(
            place_panels(3, &json!({"slots": [0, 2, 3]}), 4),
            vec![Some(0), None, Some(1), Some(2)]
        );
        assert_eq!(
            place_panels(2, &json!({"slots": [3, 1]}), 4),
            vec![None, Some(1), None, Some(0)]
        );
    }

    #[test]
    fn bad_positions_fall_back_to_the_free_slots() {
        // Out of range, duplicate, wrong type, too short: nothing is lost that fits.
        assert_eq!(
            place_panels(3, &json!({"slots": [9, 1, 1]}), 4),
            vec![Some(0), Some(1), Some(2), None]
        );
        assert_eq!(
            place_panels(2, &json!({"slots": "x"}), 2),
            vec![Some(0), Some(1)]
        );
        assert_eq!(
            place_panels(3, &json!({"slots": [2]}), 4),
            vec![Some(1), Some(2), Some(0), None]
        );
        assert_eq!(
            place_panels(2, &json!({"slots": [-1, 0.5]}), 2),
            vec![Some(0), Some(1)]
        );
    }

    #[test]
    fn saving_keeps_positions_and_the_v010_layout_key() {
        let slots = vec![
            Some(Spec::Terminal {
                title: "PowerShell".into(),
                shell: Shell::Windows,
            }),
            None,
            Some(Spec::Logcat {
                route_id: "route_1".into(),
            }),
            Some(Spec::Terminal {
                title: "SSH fedora".into(),
                shell: Shell::Ssh {
                    host_id: "host_a".into(),
                    tmux: None,
                },
            }),
        ];
        let (layout, panels) = write_workspace(Layout::Grid, &slots);
        assert_eq!(layout, json!({"layout": "grid", "slots": [0, 2, 3]}));
        assert_eq!(
            panels[0].target,
            json!({"title": "PowerShell", "shell": {"kind": "power_shell"}})
        );
        assert_eq!(panels[1], panel("logcat", json!({"route_id": "route_1"})));
        assert_eq!(
            panels[2].target,
            json!({"title": "SSH fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": null}})
        );
        // Reading it back gives the same slots.
        let (read_layout, read_slots) = read_workspace(&record(layout, panels));
        assert_eq!(read_layout, Layout::Grid);
        assert_eq!(read_slots, slots);
    }

    #[test]
    fn panels_beyond_the_layout_are_not_saved() {
        let slots = vec![
            Some(Spec::Mirror {
                route_id: "a".into(),
            }),
            Some(Spec::Mirror {
                route_id: "b".into(),
            }),
        ];
        let (layout, panels) = write_workspace(Layout::Single, &slots);
        assert_eq!(layout, json!({"layout": "single", "slots": [0]}));
        assert_eq!(panels.len(), 1);
    }

    #[test]
    fn names_are_checked() {
        assert!(name_problem("  ").contains("name"));
        assert!(name_problem(&"x".repeat(81)).contains("80"));
        assert_eq!(name_problem(" Bench "), "");
    }

    #[test]
    fn tmux_names_match_v010() {
        // (1732200000000).toString(36) in JavaScript.
        assert_eq!(tmux_name(1_732_200_000_000), "senvakara-m3rf5m2o");
        assert_eq!(tmux_name(0), "senvakara-0");
    }

    #[test]
    fn local_terminals_always_run_and_missing_targets_say_why() {
        let local = Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows,
        };
        assert_eq!(blocked(&local, &[], &[]), None);
        let gone = blocked(
            &Spec::Files {
                host_id: "host_x".into(),
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(gone.text.contains("removed"));
        assert_eq!(gone.connect, None);
        let forgotten = blocked(
            &Spec::Logcat {
                route_id: "route_x".into(),
            },
            &[],
            &[],
        )
        .unwrap();
        assert!(forgotten.text.contains("forgotten"));
    }

    #[test]
    fn the_picker_always_offers_local_shells() {
        let options = panel_options(&[], &[], &["Ubuntu".to_string()]);
        let labels: Vec<&str> = options.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(labels, ["Terminal: PowerShell", "Terminal: WSL Ubuntu"]);
    }
}
