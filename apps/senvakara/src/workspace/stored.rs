//! The stored form of a workspace: reading and writing the v0.1.0 JSON.

use senvakara_app::{PanelRecord, WorkspaceRecord};
use serde_json::{json, Value};

use super::{Layout, Shell, Spec};

/// The panel types the database accepts.
const PANEL_TYPES: [&str; 5] = ["terminal", "mirror", "desktop", "files", "logcat"];

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

#[cfg(test)]
mod tests;
