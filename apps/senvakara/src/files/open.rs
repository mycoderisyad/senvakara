//! Opening a Files tab, wiring its callbacks and choosing the place each side shows.

use std::rc::Rc;

use slint::{ComponentHandle, Model, SharedString};

use crate::{App, Files, Pane, TabInfo};

use super::{FilesPane, Kind, Side, Source};

impl App {
    pub fn init_files(self: &Rc<Self>, ui: &crate::MainWindow) {
        let files = ui.global::<Files>();
        files.on_pick_source({
            let app = self.clone();
            move |id, side, index| app.files_pick_source(id, side as usize, index)
        });
        files.on_go({
            let app = self.clone();
            move |id, side, path| app.files_go(id, side as usize, path.to_string())
        });
        files.on_up({
            let app = self.clone();
            move |id, side| app.files_up(id, side as usize)
        });
        files.on_refresh({
            let app = self.clone();
            move |id, side| app.files_refresh(id, side as usize)
        });
        files.on_open({
            let app = self.clone();
            move |id, side, row| app.files_open(id, side as usize, row as usize)
        });
        files.on_select({
            let app = self.clone();
            move |id, side, row, control, shift| {
                app.files_select(id, side as usize, row as usize, control, shift)
            }
        });
        files.on_key({
            let app = self.clone();
            move |id, side, text, control, shift| {
                app.files_key(id, side as usize, &text, control, shift)
            }
        });
        files.on_copy({
            let app = self.clone();
            move |id, side| app.files_copy(id, side as usize)
        });
    }

    /// Open a Files tab with this PC on the left and `source` on the right.
    pub fn open_files(self: &Rc<Self>, source: Source) {
        let id = self.take_id();
        let title = tab_title(
            Kind::Local,
            "This PC",
            source.kind(),
            &self.place_name(&source),
        );
        let hue = match &source {
            Source::Local => 0,
            Source::Host(host) => self.inv.hue(host),
            Source::Device(route) => self
                .inv
                .device(route)
                .map(|device| self.inv.hue(&device.record.host_id))
                .unwrap_or(0),
        };
        self.sessions
            .borrow_mut()
            .push(Pane::Files(FilesPane::new(id, source)));
        self.tabs.push(TabInfo {
            id,
            kind: "files".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
        self.files_start(id);
    }

    /// List the starting folder of both sides of a pane that was just put in place.
    pub fn files_start(self: &Rc<Self>, id: i32) {
        for side in 0..2 {
            let source = self.with_files(id, |pane| pane.sides[side].borrow().source.clone());
            let Some(source) = source else { return };
            let path = source.start_path();
            self.files_load(id, side, source, path);
        }
    }

    /// Run `f` on the Files pane with this id, if it is still open (in a tab or a workspace).
    pub(super) fn with_files<R>(&self, id: i32, f: impl FnOnce(&FilesPane) -> R) -> Option<R> {
        self.with_pane(id, |pane, _| match pane {
            Pane::Files(pane) => Some(f(pane)),
            _ => None,
        })
        .flatten()
    }

    fn files_pick_source(self: &Rc<Self>, id: i32, side: usize, index: i32) {
        let Some((id, current, _)) = self.files_side(id, side) else {
            return;
        };
        let picked = self
            .with_files(id, |pane| {
                pane.sources.borrow().get(index.max(0) as usize).cloned()
            })
            .flatten();
        let Some(source) = picked else { return };
        if source == current {
            return;
        }
        self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            let (rows, request) = (state.rows.clone(), state.request);
            rows.set_vec(Vec::new());
            let input_rev = state.input_rev.wrapping_add(1);
            *state = Side {
                rows,
                request,
                input_rev,
                ..Side::new(source.clone())
            };
        });
        self.files_retitle(id);
        let path = source.start_path();
        self.files_load(id, side, source, path);
    }

    /// Name the tab after the places it shows. A pane in a workspace panel has no tab.
    fn files_retitle(&self, id: i32) {
        let sessions = self.sessions.borrow();
        let Some(index) = sessions.iter().position(|pane| pane.id() == id) else {
            return;
        };
        let Pane::Files(pane) = &sessions[index] else {
            return;
        };
        let (left, right) = (
            pane.sides[0].borrow().source.clone(),
            pane.sides[1].borrow().source.clone(),
        );
        let title = tab_title(
            left.kind(),
            &self.place_name(&left),
            right.kind(),
            &self.place_name(&right),
        );
        // The tab carries the colour of the first machine that is not this PC.
        let hue = [&right, &left]
            .into_iter()
            .find_map(|source| match source {
                Source::Local => None,
                Source::Host(host) => Some(self.inv.hue(host)),
                Source::Device(route) => self
                    .inv
                    .device(route)
                    .map(|device| self.inv.hue(&device.record.host_id)),
            })
            .unwrap_or(0);
        if let Some(mut tab) = self.tabs.row_data(index) {
            if tab.title != title.as_str() || tab.hue != hue {
                tab.title = title.into();
                tab.hue = hue;
                self.tabs.set_row_data(index, tab);
            }
        }
    }
}

/// What a Files tab is called: the places it shows that are not this PC.
fn tab_title(left: Kind, left_name: &str, right: Kind, right_name: &str) -> String {
    match (left, right) {
        (Kind::Local, Kind::Local) => "Files, this PC".to_string(),
        (Kind::Local, _) => format!("Files, {right_name}"),
        (_, Kind::Local) => format!("Files, {left_name}"),
        _ if left_name == right_name => format!("Files, {left_name}"),
        _ => format!("Files, {left_name} and {right_name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_titles() {
        use Kind::*;
        assert_eq!(tab_title(Local, "This PC", Host, "fedora"), "Files, fedora");
        assert_eq!(tab_title(Device, "Pixel", Local, "This PC"), "Files, Pixel");
        assert_eq!(
            tab_title(Host, "fedora", Device, "Pixel"),
            "Files, fedora and Pixel"
        );
        assert_eq!(tab_title(Host, "fedora", Host, "fedora"), "Files, fedora");
        assert_eq!(
            tab_title(Local, "This PC", Local, "This PC"),
            "Files, this PC"
        );
    }
}
