//! Opening a Files tab, wiring its callbacks and naming the tab after what it shows.

use std::rc::Rc;

use slint::{ComponentHandle, Model, SharedString};

use crate::{App, Files, Pane, TabInfo};

use super::{FilesPane, Kind, Source};

impl App {
    pub fn init_files(self: &Rc<Self>, ui: &crate::MainWindow) {
        let files = ui.global::<Files>();
        files.on_pick_show({
            let app = self.clone();
            move |id, side, show| app.files_pick_show(id, side as usize, show)
        });
        files.on_pick_search({
            let app = self.clone();
            move |id, side, query| app.files_pick_search(id, side as usize, &query)
        });
        files.on_pick({
            let app = self.clone();
            move |id, side, index| app.files_pick(id, side as usize, index)
        });
        files.on_go({
            let app = self.clone();
            move |id, side, path| app.files_go(id, side as usize, path.to_string())
        });
        files.on_up({
            let app = self.clone();
            move |id, side| app.files_up(id, side as usize)
        });
        files.on_travel({
            let app = self.clone();
            move |id, side, back| app.files_travel(id, side as usize, back)
        });
        files.on_crumb({
            let app = self.clone();
            move |id, side, index| app.files_crumb(id, side as usize, index.max(0) as usize)
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
        files.on_sort({
            let app = self.clone();
            move |id, side, column| app.files_sort(id, side as usize, &column)
        });
        files.on_filter({
            let app = self.clone();
            move |id, side, text| app.files_filter(id, side as usize, &text)
        });
        files.on_new_folder({
            let app = self.clone();
            move |id, side, name| app.files_new_folder(id, side as usize, &name)
        });
        files.on_rename({
            let app = self.clone();
            move |id, side, name| app.files_rename(id, side as usize, &name)
        });
        files.on_delete_selected({
            let app = self.clone();
            move |id, side| app.files_delete_ask(id, side as usize)
        });
        files.on_chmod({
            let app = self.clone();
            move |id, side, mode| app.files_chmod(id, side as usize, mode)
        });
        files.on_toggle_hidden({
            let app = self.clone();
            move |id, side| app.files_toggle_hidden(id, side as usize)
        });
        files.on_select_all({
            let app = self.clone();
            move |id, side| app.files_select_all(id, side as usize)
        });
        files.on_close_side({
            let app = self.clone();
            move |id, side| app.files_close_side(id, side as usize)
        });
        files.on_hovered({
            let app = self.clone();
            move |id, side| app.files.hover.set((id, side as usize))
        });
        files.on_drag_data(|id, side| {
            SharedString::from(super::dragdrop::token(id, side as usize)).into()
        });
        files.on_can_drop({
            let app = self.clone();
            move |id, side, row, data| {
                let token = data.plain_text().unwrap_or_default();
                app.files_can_drop(&token, id, side as usize, row)
            }
        });
        files.on_dropped({
            let app = self.clone();
            move |id, side, row, data| {
                let token = data.plain_text().unwrap_or_default();
                app.files_dropped(&token, id, side as usize, row)
            }
        });
    }

    /// Open a Files tab with this PC on the left and `source` on the right.
    pub fn open_files(self: &Rc<Self>, source: Source) {
        let id = self.take_id();
        self.open_files_pane(FilesPane::new(id, source));
    }

    /// Open the SFTP tab: this PC on the left, and on the right the invitation to choose a
    /// host.
    pub fn open_sftp(self: &Rc<Self>) {
        let id = self.take_id();
        self.open_files_pane(FilesPane::sftp(id));
    }

    /// The pinned SFTP tab next to Home: show the SFTP pane that is open, or open one.
    pub fn sftp_tab_open(self: &Rc<Self>) {
        match self.sftp_tab() {
            Some(index) => self.select(index as i32),
            None => self.open_sftp(),
        }
    }

    /// Where the first SFTP pane is among the tabs; the tab strip shows it pinned, not in line.
    pub(crate) fn sftp_tab(&self) -> Option<usize> {
        self.sessions
            .borrow()
            .iter()
            .position(|pane| matches!(pane, Pane::Files(files) if files.sftp))
    }

    fn open_files_pane(self: &Rc<Self>, pane: FilesPane) {
        let id = pane.id;
        self.sessions.borrow_mut().push(Pane::Files(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "files".into(),
            title: SharedString::new(),
            detail: SharedString::new(),
            hue: 0,
            exited: false,
        });
        self.files_retitle(id);
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

    /// Name the tab after the places it shows. A pane in a workspace panel has no tab.
    pub(super) fn files_retitle(&self, id: i32) {
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
        let title = if pane.sftp {
            "SFTP".to_string()
        } else {
            tab_title(
                left.kind(),
                &self.place_name(&left),
                right.kind(),
                &self.place_name(&right),
            )
        };
        // The tab carries the colour of the first machine that is not this PC.
        let hue = [&right, &left]
            .into_iter()
            .find_map(|source| match source {
                Source::None | Source::Local => None,
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
        (Kind::Local | Kind::None, Kind::Local | Kind::None) => "Files, this PC".to_string(),
        (Kind::Local | Kind::None, _) => format!("Files, {right_name}"),
        (_, Kind::Local | Kind::None) => format!("Files, {left_name}"),
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
