//! What the New Tab page and the menu offer right now, read from the inventory and the shells
//! found on this PC.

use senvakara_adapters_pty::ShellKind;
use slint::Model;

use crate::palette::{Command, Entry};
use crate::{App, LaunchItem};

use super::{host_action, quick_hint, recent_order, Item, Row, RECENT, RECENT_LIMIT};

impl App {
    /// The shells of this PC, in the order they were found.
    fn launcher_shells(&self) -> Vec<Item> {
        self.launcher
            .shells
            .borrow()
            .iter()
            .map(|shell| {
                let wsl = matches!(shell.kind, ShellKind::Wsl { .. });
                Item {
                    entry: Entry {
                        title: shell.name.clone(),
                        detail: if wsl { "WSL on this PC" } else { "This PC" }.into(),
                        kind: if wsl { "linux" } else { "terminal" },
                        command: Command::Shell(shell.kind.clone(), shell.name.clone()),
                    },
                    label: String::new(),
                    // The colours the tabs of these shells always had.
                    hue: if wsl { 4 } else { 1 },
                }
            })
            .collect()
    }

    /// Every remote host that can be opened, in Home's order, with when it was last reached.
    fn launcher_hosts(&self) -> Vec<(Item, Option<String>)> {
        let cards = self.inv.host_items();
        self.inv
            .hosts()
            .into_iter()
            .zip(cards)
            .filter(|(host, _)| !host.is_local)
            .filter_map(|(host, card)| {
                let offered: Vec<String> = card
                    .actions
                    .iter()
                    .filter(|action| action.reason.is_empty())
                    .map(|action| action.id.to_string())
                    .collect();
                let offered: Vec<&str> = offered.iter().map(String::as_str).collect();
                let action = host_action(&offered)?;
                let detail = match action {
                    "trust" => format!("{}  ·  verify the host key first", card.subtitle),
                    _ => card.subtitle.to_string(),
                };
                let item = Item {
                    entry: Entry {
                        title: host.display_name.clone(),
                        detail,
                        kind: if action == "rdp" { "desktop" } else { "server" },
                        command: Command::Host(host.id.clone(), action.to_string()),
                    },
                    label: host.group_name.clone().unwrap_or_default(),
                    hue: card.hue,
                };
                Some((item, host.last_seen_at))
            })
            .collect()
    }

    /// The screen of every phone that can be used right now.
    fn launcher_devices(&self) -> Vec<Item> {
        self.inv
            .device_items()
            .into_iter()
            .filter(|device| device.primary.id == "mirror" && device.primary.reason.is_empty())
            .map(|device| Item {
                entry: Entry {
                    title: format!("Screen of {}", device.name),
                    detail: device.route.to_string(),
                    kind: "mirror",
                    command: Command::Device(device.id.to_string(), "mirror".into()),
                },
                label: String::new(),
                hue: device.host_hue,
            })
            .collect()
    }

    /// The lines of the page for `query`.
    pub(super) fn launcher_rows(&self, query: &str) -> Vec<Row> {
        let (hosts, seen): (Vec<Item>, Vec<Option<String>>) =
            self.launcher_hosts().into_iter().unzip();
        let seen: Vec<Option<&str>> = seen.iter().map(Option::as_deref).collect();
        let recent = recent_order(&seen, RECENT_LIMIT)
            .into_iter()
            .map(|index| hosts[index].clone())
            .collect();
        super::rows(
            &[
                (RECENT, recent),
                ("Hosts", hosts),
                ("Local shells", self.launcher_shells()),
                ("Phones", self.launcher_devices()),
            ],
            query,
        )
    }

    /// The menu next to "+", in the order Ctrl+Shift+1 to 9 count: the local shells as found
    /// (Windows PowerShell, PowerShell 7, Command Prompt, Git Bash, WSL distributions), then
    /// the hosts in Home's order, then the phones. Each group after the first starts with a
    /// line above it.
    pub(super) fn launcher_quick_items(&self) -> (Vec<Item>, Vec<LaunchItem>) {
        let groups = [
            self.launcher_shells(),
            self.launcher_hosts()
                .into_iter()
                .map(|(item, _)| item)
                .collect(),
            self.launcher_devices(),
        ];
        let mut items = Vec::new();
        let mut view = Vec::new();
        for group in groups {
            for (position, item) in group.into_iter().enumerate() {
                view.push(LaunchItem {
                    divider: position == 0 && !items.is_empty(),
                    label: quick_hint(items.len()).into(),
                    ..launch_item(&item)
                });
                items.push(item);
            }
        }
        (items, view)
    }
}

/// A line of the page as the view draws it.
pub(super) fn launch_row(row: &Row) -> LaunchItem {
    match row {
        Row::Header(title) => LaunchItem {
            header: true,
            title: title.as_str().into(),
            ..Default::default()
        },
        Row::Item(item) => launch_item(item),
    }
}

fn launch_item(item: &Item) -> LaunchItem {
    LaunchItem {
        header: false,
        divider: false,
        title: item.entry.title.as_str().into(),
        detail: item.entry.detail.as_str().into(),
        kind: item.entry.kind.into(),
        label: item.label.as_str().into(),
        hue: item.hue,
    }
}
