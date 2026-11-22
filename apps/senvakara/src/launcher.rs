//! The New Tab page and the menu next to "+" (TAB-NEW-01, TAB-NEW-02): hosts, the shells of
//! this PC and phones, one click or one key away.
//!
//! "+" and Ctrl+Shift+T open the page in a tab of its own; what the user picks there takes the
//! page's place in the tab strip. The menu and Ctrl+Shift+1 to 9 open the same things without
//! the page. The local shells are looked for off the UI thread, once at startup and again when
//! Home is refreshed.

mod items;
mod page;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_app::LocalShell;
use slint::VecModel;

use crate::palette::{filter_by, Entry};
use crate::LaunchItem;

/// One thing the page or the menu can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub entry: Entry,
    /// Shown at the end of the row: the parent group of a host.
    pub label: String,
    pub hue: i32,
}

/// A line of the page: a section title or something to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(String),
    Item(Item),
}

/// The section that repeats hosts from the list below it.
const RECENT: &str = "Recent connections";

/// How many hosts "Recent connections" shows.
const RECENT_LIMIT: usize = 5;

/// Positions of the hosts that were connected to before, newest first, at most `limit`.
/// `seen` holds each host's last connection time as the backend stores it (ISO 8601 in UTC,
/// which sorts as text); a host that was never reached is left out.
pub fn recent_order(seen: &[Option<&str>], limit: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..seen.len())
        .filter(|&index| seen[index].is_some())
        .collect();
    order.sort_by(|a, b| seen[*b].cmp(&seen[*a]));
    order.truncate(limit);
    order
}

/// What picking a host does, given the actions its card offers right now: a terminal, Remote
/// Desktop for an RDP host, or the host-key question for a host whose key is not trusted yet.
pub fn host_action(available: &[&str]) -> Option<&'static str> {
    ["terminal", "rdp", "trust"]
        .into_iter()
        .find(|id| available.contains(id))
}

/// The lines of the page. With nothing typed, every section that has items under its title;
/// with a query, one list of the matches, best first (the palette's rule), in which a recent
/// host appears once. The group label of a host matches too.
pub fn rows(sections: &[(&str, Vec<Item>)], query: &str) -> Vec<Row> {
    if query.trim().is_empty() {
        return sections
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .flat_map(|(title, items)| {
                std::iter::once(Row::Header(title.to_string()))
                    .chain(items.iter().cloned().map(Row::Item))
            })
            .collect();
    }
    let all: Vec<Item> = sections
        .iter()
        .filter(|(title, _)| *title != RECENT)
        .flat_map(|(_, items)| items.iter().cloned())
        .collect();
    filter_by(&all, query, |item| {
        (
            item.entry.title.clone(),
            format!("{} {}", item.entry.detail, item.label),
        )
    })
    .into_iter()
    .map(Row::Item)
    .collect()
}

/// Move the chosen line by `by` over the lines that can be chosen, skipping section titles and
/// stopping at both ends. When the current line cannot be chosen (the list changed), the
/// nearest one after it is taken, or else the last one before it.
pub fn step(rows: &[Row], current: usize, by: i32) -> usize {
    let choosable: Vec<usize> = (0..rows.len())
        .filter(|&index| matches!(rows[index], Row::Item(_)))
        .collect();
    let Some(&last) = choosable.last() else {
        return 0;
    };
    let position = match choosable.binary_search(&current) {
        Ok(position) => position as i64 + by as i64,
        // Not on an item: land on the next one, whichever way the user was going.
        Err(position) => position as i64 + (by.min(0) as i64),
    };
    choosable
        .get(position.clamp(0, choosable.len() as i64 - 1) as usize)
        .copied()
        .unwrap_or(last)
}

/// Which entry of the menu Ctrl+Shift plus this key opens: 1 is the first. With Shift held
/// the key arrives as the symbol above the digit on a US keyboard, so both are accepted.
pub fn quick_slot(key: char) -> Option<usize> {
    const SHIFTED: &str = "!@#$%^&*(";
    match key {
        '1'..='9' => Some(key as usize - '1' as usize),
        _ => SHIFTED.chars().position(|symbol| symbol == key),
    }
}

/// The hint the menu shows next to its nth entry, or "" past the ninth.
pub fn quick_hint(index: usize) -> String {
    if index < 9 {
        format!("Ctrl+Shift+{}", index + 1)
    } else {
        String::new()
    }
}

#[derive(Default)]
pub struct State {
    /// The shells found on this PC, in menu order. Empty until the first look finishes.
    shells: RefCell<Vec<LocalShell>>,
    query: RefCell<String>,
    /// The lines of the page as drawn.
    rows: RefCell<Vec<Row>>,
    current: Cell<usize>,
    /// The Remote Desktop host the page last started: its outcome is shown as a note.
    launched: RefCell<Option<String>>,
    model: Rc<VecModel<LaunchItem>>,
    /// The entries of the menu next to "+", as last shown.
    quick: RefCell<Vec<Item>>,
    quick_model: Rc<VecModel<LaunchItem>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::Command;

    fn item(title: &str, detail: &str, label: &str) -> Item {
        Item {
            entry: Entry {
                title: title.into(),
                detail: detail.into(),
                kind: "terminal",
                command: Command::Sftp,
            },
            label: label.into(),
            hue: 0,
        }
    }

    fn sample() -> Vec<(&'static str, Vec<Item>)> {
        vec![
            (RECENT, vec![item("fedora", "ssh, me", "Lab")]),
            (
                "Hosts",
                vec![
                    item("fedora", "ssh, me", "Lab"),
                    item("office pc", "rdp, admin", ""),
                ],
            ),
            ("Local shells", vec![item("Git Bash", "This PC", "")]),
            ("Phones", Vec::new()),
        ]
    }

    fn titles(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header(title) => format!("# {title}"),
                Row::Item(item) => item.entry.title.clone(),
            })
            .collect()
    }

    #[test]
    fn hosts_seen_most_recently_come_first_and_unseen_ones_are_left_out() {
        let seen = [
            Some("2026-11-20T08:00:00.000Z"),
            None,
            Some("2026-11-22T09:30:00.000Z"),
            Some("2026-11-21T23:59:59.000Z"),
        ];
        assert_eq!(recent_order(&seen, 5), [2, 3, 0]);
        assert_eq!(recent_order(&seen, 2), [2, 3]);
        assert!(recent_order(&[None, None], 5).is_empty());
    }

    #[test]
    fn picking_a_host_opens_what_its_card_offers() {
        assert_eq!(
            host_action(&["terminal", "connect", "trust"]),
            Some("terminal")
        );
        assert_eq!(host_action(&["rdp", "edit"]), Some("rdp"));
        // The key is not trusted yet: the question comes first.
        assert_eq!(host_action(&["trust", "edit"]), Some("trust"));
        assert_eq!(host_action(&["edit", "remove"]), None);
    }

    #[test]
    fn with_nothing_typed_each_section_with_items_gets_its_title() {
        assert_eq!(
            titles(&rows(&sample(), "  ")),
            [
                "# Recent connections",
                "fedora",
                "# Hosts",
                "fedora",
                "office pc",
                "# Local shells",
                "Git Bash"
            ]
        );
        assert!(rows(&[("Hosts", Vec::new())], "").is_empty());
    }

    #[test]
    fn a_query_gives_one_list_without_titles_or_repeated_hosts() {
        assert_eq!(titles(&rows(&sample(), "FED")), ["fedora"]);
        assert_eq!(titles(&rows(&sample(), "bash")), ["Git Bash"]);
        // The detail and the group label match too.
        assert_eq!(titles(&rows(&sample(), "admin")), ["office pc"]);
        assert_eq!(titles(&rows(&sample(), "lab")), ["fedora"]);
        assert!(rows(&sample(), "zzz").is_empty());
    }

    #[test]
    fn the_chosen_line_skips_titles_and_stops_at_both_ends() {
        let rows = rows(&sample(), "");
        // 0 title, 1 fedora, 2 title, 3 fedora, 4 office pc, 5 title, 6 Git Bash.
        assert_eq!(step(&rows, 1, 1), 3);
        assert_eq!(step(&rows, 3, -1), 1);
        assert_eq!(step(&rows, 1, -1), 1);
        assert_eq!(step(&rows, 6, 1), 6);
        // Standing on a title after the list changed: the next item, or the one before.
        assert_eq!(step(&rows, 0, 0), 1);
        assert_eq!(step(&rows, 2, 0), 3);
        assert_eq!(step(&rows, 2, -1), 1);
        assert_eq!(step(&rows, 99, 0), 6);
        assert_eq!(step(&[], 3, 1), 0);
    }

    #[test]
    fn ctrl_shift_digits_name_the_first_nine_menu_entries() {
        assert_eq!(quick_slot('1'), Some(0));
        assert_eq!(quick_slot('9'), Some(8));
        assert_eq!(quick_slot('!'), Some(0));
        assert_eq!(quick_slot('('), Some(8));
        assert_eq!(quick_slot('0'), None);
        assert_eq!(quick_slot('t'), None);
        assert_eq!(quick_hint(0), "Ctrl+Shift+1");
        assert_eq!(quick_hint(8), "Ctrl+Shift+9");
        assert_eq!(quick_hint(9), "");
    }
}
