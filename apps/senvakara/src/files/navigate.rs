//! Listing a folder into a side and moving from one folder to another.

use std::rc::Rc;

use crate::inventory::friendly;
use crate::App;

use super::entries::{list_local, sort_entries, Entry};
use super::paths::{join, parent, tidy_local, tidy_posix};
use super::{Kind, Source};

impl App {
    /// Say what the folder field of a side should show.
    fn set_input(&self, id: i32, side: usize, text: &str) {
        self.with_files(id, |pane| pane.sides[side].borrow_mut().set_input(text));
        self.files_publish(id);
    }

    /// List `path` of `source` into a side. The rows on screen stay until the answer is in.
    pub(super) fn files_load(self: &Rc<Self>, id: i32, side: usize, source: Source, path: String) {
        let request = self.with_files(id, |pane| {
            let mut state = pane.sides[side].borrow_mut();
            state.loading = true;
            state.error.clear();
            state.request += 1;
            state.request
        });
        let Some(request) = request else { return };
        self.files_publish(id);

        let backend = self.backend.clone();
        let place = source.clone();
        self.run(
            async move {
                match place {
                    Source::Local => tokio::task::spawn_blocking(move || list_local(&path))
                        .await
                        .unwrap_or_else(|_| Err("The folder could not be read.".into())),
                    Source::Host(host) => backend
                        .sftp_list(host, path)
                        .await
                        .map(|listing| {
                            let mut entries: Vec<Entry> = listing
                                .entries
                                .into_iter()
                                .filter(|entry| entry.name != "." && entry.name != "..")
                                .map(|entry| Entry {
                                    name: entry.name,
                                    is_dir: entry.is_dir,
                                    is_symlink: entry.is_symlink,
                                    is_drive: false,
                                    size: entry.size,
                                    modified: entry.modified.map(i64::from),
                                })
                                .collect();
                            sort_entries(&mut entries);
                            (listing.path, entries)
                        })
                        .map_err(|error| friendly(&error)),
                    Source::Device(route) => {
                        let shown = tidy_posix(&path);
                        backend
                            .device_files_list(route, path)
                            .await
                            .map(|list| {
                                let mut entries: Vec<Entry> = list
                                    .into_iter()
                                    .filter(|entry| entry.name != "." && entry.name != "..")
                                    .map(|entry| Entry {
                                        name: entry.name,
                                        is_dir: entry.is_dir,
                                        is_symlink: entry.is_symlink,
                                        is_drive: false,
                                        size: entry.size as u64,
                                        modified: Some(entry.mtime as i64),
                                    })
                                    .collect();
                                sort_entries(&mut entries);
                                (shown, entries)
                            })
                            .map_err(|error| friendly(&error))
                    }
                }
            },
            move |app, result| {
                let fresh = app.with_files(id, |pane| {
                    let mut state = pane.sides[side].borrow_mut();
                    if state.request != request || state.source != source {
                        return None;
                    }
                    state.loading = false;
                    match result {
                        Ok((path, entries)) => {
                            state.show(path, entries);
                            Some(state.path.clone())
                        }
                        Err(message) => {
                            state.error = message;
                            None
                        }
                    }
                });
                if let Some(Some(path)) = fresh {
                    app.set_input(id, side, &path);
                    // Listing a host that was disconnected connects it again: Home should say so.
                    if let Source::Host(host) = &source {
                        if !app.inv.connected_hosts().contains(host) {
                            app.reload();
                        }
                    }
                }
                app.files_publish(id);
            },
        );
    }

    /// (pane id, source, folder on screen) of a side of a pane.
    pub(super) fn files_side(&self, id: i32, side: usize) -> Option<(i32, Source, String)> {
        self.with_files(id, |pane| {
            let state = pane.sides.get(side)?.borrow();
            Some((id, state.source.clone(), state.path.clone()))
        })
        .flatten()
    }

    pub(super) fn files_go(self: &Rc<Self>, id: i32, side: usize, path: String) {
        let Some((id, source, _)) = self.files_side(id, side) else {
            return;
        };
        let path = match source.kind() {
            Kind::Local => tidy_local(&path),
            Kind::Host if path.trim().is_empty() => ".".into(),
            Kind::Device if path.trim().is_empty() => "/".into(),
            _ => path.trim().to_string(),
        };
        self.files_load(id, side, source, path);
    }

    pub(super) fn files_up(self: &Rc<Self>, id: i32, side: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        if let Some(above) = parent(source.kind(), &path) {
            self.files_load(id, side, source, above);
        }
    }

    pub(super) fn files_refresh(self: &Rc<Self>, id: i32, side: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        let loaded = self
            .with_files(id, |pane| pane.sides[side].borrow().loaded)
            .unwrap_or(false);
        let path = if loaded { path } else { source.start_path() };
        self.files_load(id, side, source, path);
    }

    pub(super) fn files_open(self: &Rc<Self>, id: i32, side: usize, row: usize) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        let name = self
            .with_files(id, |pane| {
                let state = pane.sides[side].borrow();
                state
                    .entries
                    .get(row)
                    .filter(|entry| entry.opens())
                    .map(|entry| entry.name.clone())
            })
            .flatten();
        if let Some(name) = name {
            let target = join(source.kind(), &path, &name);
            self.files_load(id, side, source, target);
        }
    }
}
