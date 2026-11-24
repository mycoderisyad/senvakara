//! Making a new folder in the folder a side shows: on this PC and on a host.

use std::rc::Rc;

use crate::inventory::friendly_with;
use crate::App;

use super::local::TAKEN;
use super::paths::{folder_name_problem, join};
use super::{Kind, Source};

/// Why a side cannot get a new folder at all, or "" when it can. `top` is the list of drives.
pub(super) fn new_folder_problem(kind: Kind, loaded: bool, top: bool) -> &'static str {
    match kind {
        Kind::None => "Select a host first.",
        _ if !loaded => "No folder is open here yet.",
        Kind::Device => "Folders cannot be created on a phone from here yet.",
        Kind::Local if top => "Open a drive first. A folder cannot be made in the list of drives.",
        _ => "",
    }
}

impl App {
    /// Make the folder `name` in the folder on screen, then list it again.
    pub(super) fn files_new_folder(self: &Rc<Self>, id: i32, side: usize, name: &str) {
        let Some((id, source, path)) = self.files_side(id, side) else {
            return;
        };
        let loaded = self
            .with_files(id, |pane| pane.sides[side].borrow().loaded)
            .unwrap_or(false);
        let kind = source.kind();
        let mut problem = new_folder_problem(kind, loaded, kind == Kind::Local && path.is_empty());
        if problem.is_empty() {
            problem = folder_name_problem(kind, name);
        }
        if !problem.is_empty() {
            self.files_fail(id, side, problem.to_string());
            return;
        }
        let name = name.trim().to_string();
        let backend = self.backend.clone();
        let (place, folder) = (source.clone(), path.clone());
        self.run(
            async move {
                match place {
                    Source::Local => {
                        let target = join(Kind::Local, &folder, &name);
                        tokio::task::spawn_blocking(move || std::fs::create_dir(target))
                            .await
                            .map_err(|_| "The folder could not be made.".to_string())?
                            .map_err(|error| match error.kind() {
                                std::io::ErrorKind::AlreadyExists => TAKEN.to_string(),
                                std::io::ErrorKind::PermissionDenied => {
                                    "Windows does not allow making a folder here.".to_string()
                                }
                                _ => format!("The folder could not be made. ({error})"),
                            })
                    }
                    Source::Host(host) => backend
                        .sftp_mkdir(host, folder, name)
                        .await
                        .map(|_| ())
                        .map_err(|error| friendly_with(&error, TAKEN)),
                    Source::None | Source::Device(_) => Ok(()),
                }
            },
            move |app, result| app.files_changed(id, side, source, path, result),
        );
    }

    /// Show an error on a side.
    pub(super) fn files_fail(&self, id: i32, side: usize, message: String) {
        self.with_files(id, |pane| {
            if let Some(state) = pane.sides.get(side) {
                state.borrow_mut().error = message;
            }
        });
        self.files_publish(id);
    }
}
