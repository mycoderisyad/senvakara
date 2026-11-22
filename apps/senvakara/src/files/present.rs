//! Presenting a Files pane to the view: the names of places, the picker and both sides.

use slint::{Model, SharedString};

use crate::inventory::transport_label;
use crate::{App, FileSide, Pane, Place};

use super::copy::copy_problem;
use super::entries::{size_text, Entry};
use super::paths::parent;
use super::{FilesPane, Kind, Source};

impl App {
    /// What a place is called in the picker, in labels and in confirmations.
    pub(super) fn place_name(&self, source: &Source) -> String {
        match source {
            Source::Local => "This PC".into(),
            Source::Host(id) => self
                .inv
                .host_name(id)
                .unwrap_or_else(|| "Unknown host".into()),
            Source::Device(route) => self
                .inv
                .device(route)
                .map(|device| device.record.display_name)
                .unwrap_or_else(|| "Unknown phone".into()),
        }
    }

    /// Redraw the pane if it is on screen.
    pub(super) fn files_publish(&self, id: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        self.with_pane(id, |pane, place| {
            if let (Pane::Files(pane), true) = (pane, self.on_screen(place)) {
                self.present_files(&ui, pane, place, false);
            }
        });
    }

    /// The places the picker offers: this PC, connected hosts, available phones, and whatever
    /// a side is showing even if it has gone away since.
    fn files_sources(&self, pane: &FilesPane) -> Vec<Source> {
        let mut sources = vec![Source::Local];
        sources.extend(self.inv.connected_hosts().into_iter().map(Source::Host));
        sources.extend(self.inv.available_devices().into_iter().map(Source::Device));
        for side in &pane.sides {
            let source = side.borrow().source.clone();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        sources
    }

    fn source_label(&self, source: &Source) -> String {
        match source {
            Source::Local => "This PC".into(),
            Source::Host(id) => {
                let name = self.place_name(source);
                if self.inv.connected_hosts().contains(id) {
                    name
                } else {
                    format!("{name} (not connected)")
                }
            }
            Source::Device(route) => match self.inv.device(route) {
                Some(device) => {
                    let record = &device.record;
                    let reach = format!(
                        "{} > {}",
                        record.host_name,
                        transport_label(&record.connection_type)
                    );
                    if record.state == "available" {
                        format!("{} ({reach})", record.display_name)
                    } else {
                        format!("{} (not connected)", record.display_name)
                    }
                }
                None => "Unknown phone (not connected)".into(),
            },
        }
    }

    /// Draw a Files pane: both sides, their rows and what can be copied.
    pub fn present_files(
        &self,
        ui: &crate::MainWindow,
        pane: &FilesPane,
        place: Place,
        full: bool,
    ) {
        let sources = self.files_sources(pane);
        let labels: Vec<SharedString> = sources
            .iter()
            .map(|source| self.source_label(source).into())
            .collect();
        for index in 0..2 {
            let side = pane.sides[index].borrow();
            let other = pane.sides[1 - index].borrow();
            let selected: Vec<&Entry> = side
                .selected
                .iter()
                .filter_map(|row| side.entries.get(*row))
                .collect();
            let reason = copy_problem(
                side.source.kind(),
                other.source.kind(),
                &selected,
                other.loaded,
                &other.path,
                self.phone_ready(&side.source) && self.phone_ready(&other.source),
            );
            // A folder's own size says nothing about what is in it: only files count.
            let bytes: u64 = selected
                .iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.size)
                .sum();
            let count = side.entries.len();
            let mut summary = format!("{count} {}", if count == 1 { "item" } else { "items" });
            if !selected.is_empty() {
                summary.push_str(&format!(
                    ", {} selected ({})",
                    selected.len(),
                    size_text(bytes)
                ));
            }
            let place = self.place_name(&side.source);
            let top = side.source.kind() == Kind::Local && side.path.is_empty();
            let info = FileSide {
                place: place.as_str().into(),
                path: if top {
                    "Drives of this PC".into()
                } else {
                    side.path.as_str().into()
                },
                can_up: side.loaded && parent(side.source.kind(), &side.path).is_some(),
                loading: side.loading,
                loaded: side.loaded,
                error: side.error.as_str().into(),
                summary: if side.loaded {
                    summary.into()
                } else {
                    SharedString::new()
                },
                copy_reason: reason.into(),
                copy_target: format!(
                    "To {}: {}",
                    self.place_name(&other.source),
                    if other.path.is_empty() {
                        "no folder open"
                    } else {
                        &other.path
                    }
                )
                .into(),
                cursor: side.cursor.map(|row| row as i32).unwrap_or(-1),
                can_open: selected.len() == 1 && selected[0].opens(),
                source_index: sources
                    .iter()
                    .position(|source| *source == side.source)
                    .unwrap_or(0) as i32,
                input: side.input.as_str().into(),
                input_rev: side.input_rev,
            };
            if pane.info.row_data(index).as_ref() != Some(&info) {
                pane.info.set_row_data(index, info);
            }
        }
        *pane.sources.borrow_mut() = sources;
        let current: Vec<SharedString> = pane.labels.iter().collect();
        if current != labels {
            pane.labels.set_vec(labels);
        }
        // Everything above is a model of the pane: the view's data is only set when it starts
        // showing this pane.
        if full {
            self.panel_update(ui, place, true, |panel| {
                panel.kind = "files".into();
                panel.files = crate::FilesData {
                    id: pane.id,
                    sources: pane.labels.clone().into(),
                    sides: pane.info.clone().into(),
                    left_rows: pane.sides[0].borrow().rows.clone().into(),
                    right_rows: pane.sides[1].borrow().rows.clone().into(),
                };
            });
        }
    }

    /// Hosts or phones changed: the pickers and the copy buttons of every pane on screen
    /// follow.
    pub fn files_inventory_changed(&self) {
        let mut ids = Vec::new();
        self.each_pane(|pane, place| {
            if let (Pane::Files(pane), true) = (pane, self.on_screen(place)) {
                ids.push(pane.id);
            }
        });
        for id in ids {
            self.files_publish(id);
        }
    }
}
