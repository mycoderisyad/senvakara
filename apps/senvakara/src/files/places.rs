//! Choosing what a side shows: the list of places inside the pane ("Select host"), with a
//! search field, the hosts reached over SSH, the phones that are available, and this PC.
//! Choosing a host that is not connected connects it; one whose key is not trusted yet shows
//! the host-key dialog first.

use std::rc::Rc;

use senvakara_app::Protocol;

use crate::inventory::{host_subtitle, transport_label};
use crate::{App, FilePlace};

use super::{FilesPane, Kind, Source};

/// One place of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Place {
    pub source: Source,
    pub title: String,
    /// "ssh, bob" for a host, the route for a phone.
    pub detail: String,
    /// "Hosts" or "Phones": drawn above the first place of each group.
    pub group: &'static str,
    pub hue: i32,
    /// A host with a live connection.
    pub connected: bool,
}

/// The places whose name or detail contains every word of `query` (case does not matter),
/// with the group title kept on the first one of each group.
pub(super) fn filter_places(places: Vec<Place>, query: &str) -> Vec<Place> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect();
    places
        .into_iter()
        .filter(|place| {
            let text = format!("{} {}", place.title, place.detail).to_lowercase();
            words.iter().all(|word| text.contains(word))
        })
        .collect()
}

/// The rows of the list: the group title shows on the first place of a group only.
fn place_rows(places: &[Place]) -> Vec<FilePlace> {
    let mut last = "";
    places
        .iter()
        .map(|place| {
            let heading = if place.group == last { "" } else { place.group };
            last = place.group;
            FilePlace {
                heading: heading.into(),
                title: place.title.as_str().into(),
                detail: place.detail.as_str().into(),
                kind: if place.source.kind() == Kind::Device {
                    "phone"
                } else {
                    "host"
                }
                .into(),
                hue: place.hue,
                connected: place.connected,
            }
        })
        .collect()
}

impl App {
    /// Every place a side can show besides this PC: the SSH hosts, connected or not, then the
    /// phones that can be used right now.
    fn files_places(&self) -> Vec<Place> {
        let hosts = self
            .inv
            .hosts()
            .into_iter()
            .filter(|host| !host.is_local && host.protocol == Protocol::Ssh)
            .map(|host| {
                let username = host.ssh.as_ref().map(|ssh| ssh.username.as_str());
                Place {
                    title: host.display_name.clone(),
                    detail: host_subtitle(Protocol::Ssh, username.unwrap_or_default()),
                    group: "Hosts",
                    hue: self.inv.hue(&host.id),
                    connected: host.connected,
                    source: Source::Host(host.id),
                }
            });
        let phones = self
            .inv
            .available_devices()
            .into_iter()
            .filter_map(|route| {
                let record = self.inv.device(&route)?.record;
                Some(Place {
                    title: record.display_name.clone(),
                    detail: format!(
                        "phone, {} > {}",
                        record.host_name,
                        transport_label(&record.connection_type)
                    ),
                    group: "Phones",
                    hue: self.inv.hue(&record.host_id),
                    connected: false,
                    source: Source::Device(route),
                })
            });
        hosts.chain(phones).collect()
    }

    /// Fill the list of places of every side that shows it.
    pub(super) fn files_fill_places(&self, pane: &FilesPane) {
        for side in &pane.sides {
            let mut side = side.borrow_mut();
            if !side.picking {
                continue;
            }
            let places = filter_places(self.files_places(), &side.query);
            let rows = place_rows(&places);
            if slint::Model::iter(&*side.place_rows).ne(rows.iter().cloned()) {
                side.place_rows.set_vec(rows);
            }
            side.places = places.into_iter().map(|place| place.source).collect();
        }
    }

    /// Show the list of places on a side, or the files again.
    pub(super) fn files_pick_show(&self, id: i32, side: usize, show: bool) {
        self.with_files(id, |pane| {
            let Some(state) = pane.sides.get(side) else {
                return;
            };
            let mut state = state.borrow_mut();
            state.picking = show;
            state.query.clear();
        });
        self.files_publish(id);
    }

    pub(super) fn files_pick_search(&self, id: i32, side: usize, query: &str) {
        self.with_files(id, |pane| {
            if let Some(state) = pane.sides.get(side) {
                state.borrow_mut().query = query.to_string();
            }
        });
        self.files_publish(id);
    }

    /// A place of the list was chosen. `index` -1 is this PC.
    pub(super) fn files_pick(self: &Rc<Self>, id: i32, side: usize, index: i32) {
        let source = self
            .with_files(id, |pane| {
                let state = pane.sides.get(side)?.borrow();
                if index < 0 {
                    Some(Source::Local)
                } else {
                    state.places.get(index as usize).cloned()
                }
            })
            .flatten();
        if let Some(source) = source {
            self.files_show_source(id, side, source);
        }
    }

    /// Make a side show `source` and list its starting folder. A host whose key is not trusted
    /// yet is asked about first; its files are listed once the key is accepted.
    pub(super) fn files_show_source(self: &Rc<Self>, id: i32, side: usize, source: Source) {
        let untrusted = match &source {
            Source::Host(host) => self
                .inv
                .host(host)
                .is_some_and(|host| host.trust_state != "trusted"),
            _ => false,
        };
        let changed = self.with_files(id, |pane| {
            let Some(state) = pane.sides.get(side) else {
                return false;
            };
            let mut state = state.borrow_mut();
            if state.source == source && !state.awaiting_trust {
                state.picking = false;
                return false;
            }
            state.switch(source.clone());
            if untrusted {
                state.awaiting_trust = true;
                state.note = "Check the key of this host in the dialog. Its files are listed once the key is trusted. If no dialog appears, the host did not answer: see its card on Home.".into();
            }
            true
        });
        self.files_retitle(id);
        if changed != Some(true) {
            self.files_publish(id);
            return;
        }
        match &source {
            Source::Host(host) if untrusted => {
                self.files_publish(id);
                self.host_action(host, "trust");
            }
            _ => {
                let path = source.start_path();
                self.files_load(id, side, source, path);
            }
        }
    }

    /// Hosts changed: a side that waited for a host key lists the host once the key is
    /// trusted.
    pub(super) fn files_trust_arrived(self: &Rc<Self>) {
        let mut ready = Vec::new();
        self.each_pane(|pane, _| {
            let crate::Pane::Files(pane) = pane else {
                return;
            };
            for (index, side) in pane.sides.iter().enumerate() {
                let mut side = side.borrow_mut();
                let Source::Host(host) = &side.source else {
                    continue;
                };
                let trusted = self
                    .inv
                    .host(host)
                    .is_some_and(|host| host.trust_state == "trusted");
                if side.awaiting_trust && trusted {
                    side.awaiting_trust = false;
                    side.note.clear();
                    ready.push((pane.id, index, side.source.clone()));
                }
            }
        });
        for (id, side, source) in ready {
            let path = source.start_path();
            self.files_load(id, side, source, path);
        }
    }
}
