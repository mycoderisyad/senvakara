//! Drag and drop (FILE-DND-01). Rows dragged inside the app carry a token that names the list
//! they come from; dropping them on a list, or on a folder of one, copies the rows selected
//! there through the same checks as the Copy button. Files dropped from Explorer are uploaded
//! to the host or phone a side shows.

use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::jobs::file_name;
use crate::{App, Files, Pane};

use super::copy::{Batch, Item};
use super::paths::parent;
use super::{pair_supported, Kind, Source};

const PREFIX: &str = "senvakara-files";

/// What a drag of rows carries: the list they are in. The rows themselves are whatever is
/// selected there when the drop lands, so the token holds no path.
pub(super) fn token(pane: i32, side: usize) -> String {
    format!("{PREFIX}:{pane}:{side}")
}

/// The list a token names, if it is one of ours.
pub(super) fn parse_token(token: &str) -> Option<(i32, usize)> {
    let mut parts = token.split(':');
    if parts.next()? != PREFIX {
        return None;
    }
    let pane = parts.next()?.parse().ok()?;
    let side = parts.next()?.parse().ok().filter(|side| *side < 2)?;
    parts.next().is_none().then_some((pane, side))
}

/// The side that takes files dropped from Explorer: the one the pointer was over last if it
/// shows a host or a phone, else the other one if that does. `kinds` is what the two sides
/// show.
pub(super) fn explorer_side(kinds: [Kind; 2], hovered: usize) -> Option<usize> {
    let hovered = hovered.min(1);
    [hovered, 1 - hovered]
        .into_iter()
        .find(|side| matches!(kinds[*side], Kind::Host | Kind::Device))
}

impl App {
    /// Whether rows dragged from the list `token` names may be dropped on a side (`row` -1) or
    /// on the folder on row `row` of it. Only says where a drop makes sense at all; why a
    /// drop then did nothing is told when it lands.
    pub(super) fn files_can_drop(&self, token: &str, id: i32, side: usize, row: i32) -> bool {
        let Some(from) = parse_token(token) else {
            return false;
        };
        let kind = |pane: i32, side: usize| {
            self.with_files(pane, |pane| {
                Some(pane.sides.get(side)?.borrow().source.kind())
            })
            .flatten()
        };
        let (Some(from_kind), Some(to_kind)) = (kind(from.0, from.1), kind(id, side)) else {
            return false;
        };
        if !pair_supported(from_kind, to_kind) {
            return false;
        }
        if row < 0 {
            // A list does not take its own rows: that would copy them onto themselves.
            return from != (id, side);
        }
        self.with_files(id, |pane| {
            let Some(state) = pane.sides.get(side) else {
                return false;
            };
            let state = state.borrow();
            state
                .entries
                .get(row as usize)
                .is_some_and(|entry| entry.holds_files())
                && !(from == (id, side) && state.selected.contains(&(row as usize)))
        })
        .unwrap_or(false)
    }

    /// Rows were dropped on a side or on a folder of it: copy them there. True when the copy
    /// was started (or is waiting for a confirmation).
    pub(super) fn files_dropped(
        self: &Rc<Self>,
        token: &str,
        id: i32,
        side: usize,
        row: i32,
    ) -> bool {
        let Some(from) = parse_token(token) else {
            return false;
        };
        if !self.files_can_drop(token, id, side, row) {
            return false;
        }
        let into = (row >= 0).then_some(row as usize);
        match self.files_plan(from, (id, side), into) {
            Ok(batch) => {
                self.files_note(id, side, "");
                self.files_send(batch);
                true
            }
            Err(problem) => {
                self.files_note(id, side, problem);
                false
            }
        }
    }

    /// Say something on a side that is not an error, or take it away with "".
    pub(super) fn files_note(&self, id: i32, side: usize, note: &str) {
        self.with_files(id, |pane| {
            if let Some(state) = pane.sides.get(side) {
                state.borrow_mut().note = note.to_string();
            }
        });
        self.files_publish(id);
    }

    /// The Files pane in the tab in front and the side of it that would take files dropped
    /// from Explorer (None when neither side shows a host or a phone).
    fn files_explorer_target(&self) -> Option<(i32, usize, Option<usize>)> {
        let sessions = self.sessions.borrow();
        let Pane::Files(pane) = sessions.get(usize::try_from(self.active.get()).ok()?)? else {
            return None;
        };
        let kinds = [0, 1].map(|side| pane.sides[side].borrow().source.kind());
        let (hover_pane, hover_side) = self.files.hover.get();
        let hovered = if hover_pane == pane.id { hover_side } else { 1 };
        Some((pane.id, hovered.min(1), explorer_side(kinds, hovered)))
    }

    /// Whether a Files tab is in front: files dropped from Explorer then go to it.
    pub fn files_in_front(&self) -> bool {
        self.files_explorer_target().is_some()
    }

    /// Files from Explorer are held over the window, or no longer: mark the side that would
    /// take them.
    pub fn files_explorer_hover(&self, hovering: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let target = self
            .files_explorer_target()
            .filter(|_| hovering)
            .and_then(|(pane, _, side)| Some((pane, side? as i32)));
        let files = ui.global::<Files>();
        let (pane, side) = target.unwrap_or((-1, -1));
        files.set_explorer_pane(pane);
        files.set_explorer_side(side);
    }

    /// Files and folders were dropped from Explorer on the Files tab in front: upload them to
    /// the folder of the side that shows a host or a phone.
    pub fn files_explorer_dropped(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let Some((id, hovered, side)) = self.files_explorer_target() else {
            return;
        };
        let Some(side) = side else {
            self.files_note(
                id,
                hovered,
                "Files dropped from Explorer are uploaded to a host or a phone. Select one on either side first.",
            );
            return;
        };
        let target = self
            .with_files(id, |pane| {
                let state = pane.sides[side].borrow();
                (state.source.clone(), state.path.clone(), state.loaded)
            })
            .filter(|(_, _, loaded)| *loaded);
        let Some((to, to_dir, _)) = target else {
            self.files_note(
                id,
                side,
                "No folder is open here yet. Wait for the list, then drop the files again.",
            );
            return;
        };
        if !self.phone_ready(&to) {
            self.files_note(
                id,
                side,
                "The phone must be connected and authorized first.",
            );
            return;
        }
        // Whether a path is a folder is asked of the disk, off the UI thread.
        self.run(
            async move {
                tokio::task::spawn_blocking(move || {
                    paths
                        .into_iter()
                        .filter_map(|path| {
                            let folder = path.is_dir();
                            Some((path.to_str()?.to_string(), folder))
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default()
            },
            move |app, dropped: Vec<(String, bool)>| {
                if dropped.is_empty() {
                    return;
                }
                if to.kind() == Kind::Device && dropped.iter().any(|(_, folder)| *folder) {
                    app.files_note(
                        id,
                        side,
                        "Folders cannot be copied to a phone yet. Drop files only.",
                    );
                    return;
                }
                let from_dir = parent(Kind::Local, &dropped[0].0).unwrap_or_default();
                let items = dropped
                    .into_iter()
                    .map(|(path, _)| Item {
                        name: file_name(&path),
                        path,
                    })
                    .collect();
                app.files_note(id, side, "");
                app.files_send(Batch {
                    from: Source::Local,
                    from_dir,
                    to,
                    to_dir,
                    items,
                });
            },
        );
    }
}
