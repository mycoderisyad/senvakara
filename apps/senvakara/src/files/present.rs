//! Presenting a Files pane to the view: the names of places, the path bar and both sides.

use std::rc::Rc;

use slint::{Model, SharedString};

use crate::{App, FileCrumb, FileSide, Pane, Place};

use super::copy::{copy_problem, Route};
use super::entries::{size_text, Entry};
use super::folders::new_folder_problem;
use super::paths::{crumbs, parent, same_dir};
use super::{FilesPane, Kind, Source};

impl App {
    /// What a place is called in the pane, in labels and in confirmations.
    pub(super) fn place_name(&self, source: &Source) -> String {
        match source {
            Source::None => "No host".into(),
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

    /// The colour of the machine a side shows.
    fn place_hue(&self, source: &Source) -> i32 {
        match source {
            Source::None | Source::Local => 0,
            Source::Host(host) => self.inv.hue(host),
            Source::Device(route) => self
                .inv
                .device(route)
                .map(|device| self.inv.hue(&device.record.host_id))
                .unwrap_or(0),
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
        self.files_fill_places(pane);
        for index in 0..2 {
            let mut side = pane.sides[index].borrow_mut();
            let trail = crumbs(side.source.kind(), &side.path);
            let trail = if side.loaded { trail } else { Vec::new() };
            let (labels, paths): (Vec<String>, Vec<String>) = trail.into_iter().unzip();
            if side.crumb_paths != paths {
                side.crumbs.set_vec(
                    labels
                        .iter()
                        .map(|label| FileCrumb {
                            label: label.as_str().into(),
                        })
                        .collect::<Vec<_>>(),
                );
                side.crumb_paths = paths;
            }
            drop(side);

            let side = pane.sides[index].borrow();
            let other = pane.sides[1 - index].borrow();
            let selected: Vec<&Entry> = side
                .selected
                .iter()
                .filter_map(|row| side.entries.get(*row))
                .collect();
            let reason = copy_problem(
                Route {
                    from: side.source.kind(),
                    to: other.source.kind(),
                    to_loaded: other.loaded,
                    to_path: &other.path,
                    phone_ready: self.phone_ready(&side.source) && self.phone_ready(&other.source),
                    same_folder: side.source == other.source
                        && same_dir(side.source.kind(), &side.path, &other.path),
                },
                &selected,
            );
            // A folder's own size says nothing about what is in it: only files count.
            let bytes: u64 = selected
                .iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.size)
                .sum();
            let count = side.entries.len();
            let mut summary = format!("{count} {}", if count == 1 { "item" } else { "items" });
            if side.entries.len() != side.all.len() {
                summary.push_str(&format!(" of {}", side.all.len()));
            }
            if !selected.is_empty() {
                summary.push_str(&format!(
                    ", {} selected ({})",
                    selected.len(),
                    size_text(bytes)
                ));
            }
            let kind = side.source.kind();
            let top = kind == Kind::Local && side.path.is_empty();
            let info = FileSide {
                place: self.place_name(&side.source).into(),
                place_kind: match kind {
                    Kind::None => "",
                    Kind::Local => "local",
                    Kind::Host => "host",
                    Kind::Device => "phone",
                }
                .into(),
                hue: self.place_hue(&side.source),
                path: if top {
                    "Drives of this PC".into()
                } else {
                    side.path.as_str().into()
                },
                can_up: side.loaded && parent(kind, &side.path).is_some(),
                can_back: !side.back.is_empty(),
                can_forward: !side.forward.is_empty(),
                loading: side.loading,
                loaded: side.loaded,
                error: side.error.as_str().into(),
                note: side.note.as_str().into(),
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
                input: side.input.as_str().into(),
                input_rev: side.input_rev,
                filter_rev: side.filter_rev,
                filtered: !side.filter.trim().is_empty(),
                sort_column: side.sort.column.id().into(),
                sort_descending: side.sort.descending,
                tall: side.tall(),
                picking: side.picking,
                new_folder_reason: new_folder_problem(kind, side.loaded, top).into(),
            };
            if pane.info.row_data(index).as_ref() != Some(&info) {
                pane.info.set_row_data(index, info);
            }
        }
        // Everything above is a model of the pane: the view's data is only set when it starts
        // showing this pane.
        if full {
            let (left, right) = (pane.sides[0].borrow(), pane.sides[1].borrow());
            self.panel_update(ui, place, true, |panel| {
                panel.kind = "files".into();
                panel.files = crate::FilesData {
                    id: pane.id,
                    sides: pane.info.clone().into(),
                    left_rows: left.rows.clone().into(),
                    right_rows: right.rows.clone().into(),
                    left_crumbs: left.crumbs.clone().into(),
                    right_crumbs: right.crumbs.clone().into(),
                    left_places: left.place_rows.clone().into(),
                    right_places: right.place_rows.clone().into(),
                };
            });
        }
    }

    /// Hosts or phones changed: the lists of places and the copy buttons of every pane on
    /// screen follow, and a side that waited for a host key goes on.
    pub fn files_inventory_changed(self: &Rc<Self>) {
        self.files_trust_arrived();
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
