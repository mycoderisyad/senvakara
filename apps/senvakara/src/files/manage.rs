//! Managing what a side of a Files pane lists: renaming an item, deleting items (after a
//! confirmation), changing the permissions of an item on a host, showing or hiding hidden
//! items, selecting everything, and closing the side.

use std::rc::Rc;

use crate::inventory::{fact, friendly, friendly_with};
use crate::{App, Pending, Question};

use super::entries::Entry;
use super::local::{delete_local, rename_local, TAKEN};
use super::paths::{item_name_problem, join};
use super::{Kind, Source};

/// Why the selection cannot be renamed, or "" when it can. `top` is the list of drives.
pub(super) fn rename_problem(
    kind: Kind,
    loaded: bool,
    top: bool,
    selected: &[&Entry],
) -> &'static str {
    match kind {
        Kind::None => "Select a host first.",
        _ if !loaded => "No folder is open here yet.",
        Kind::Device => "Renaming items on a phone is not supported here yet.",
        Kind::Local if top => "Drives cannot be renamed.",
        _ if selected.is_empty() => "Select one file or folder to rename.",
        _ if selected.len() > 1 => "Select only one item to rename.",
        _ if selected[0].is_drive => "Drives cannot be renamed.",
        _ => "",
    }
}

/// Why the selection cannot be deleted, or "" when it can. `top` is the list of drives.
pub(super) fn delete_problem(
    kind: Kind,
    loaded: bool,
    top: bool,
    selected: &[&Entry],
) -> &'static str {
    match kind {
        Kind::None => "Select a host first.",
        _ if !loaded => "No folder is open here yet.",
        Kind::Device => "Deleting items on a phone is not supported here yet.",
        Kind::Local if top => "Drives cannot be deleted.",
        _ if selected.is_empty() => "Select one or more files or folders to delete.",
        _ if selected.iter().any(|entry| entry.is_drive) => "Drives cannot be deleted.",
        _ => "",
    }
}

/// Why the permissions of the selection cannot be edited, or "" when they can.
pub(super) fn chmod_problem(kind: Kind, loaded: bool, selected: &[&Entry]) -> &'static str {
    match kind {
        Kind::Host if !loaded => "No folder is open here yet.",
        Kind::Host if selected.is_empty() => "Select one file or folder first.",
        Kind::Host if selected.len() > 1 => "Select only one item to edit permissions.",
        Kind::Host if selected[0].is_symlink => "Permissions cannot be edited on a symbolic link.",
        Kind::Host => "",
        _ => "Permissions can only be edited on a remote SSH host.",
    }
}

/// The permission bits (`0o755`) of an `ls -l` permission string, with or without the type
/// letter in front (`"drwxr-xr-x"`, `"rwxr-xr-x"`); `None` when the text is not one.
pub(super) fn parse_permissions(text: &str) -> Option<u32> {
    let letters: Vec<char> = text.trim().chars().collect();
    let bits = match letters.len() {
        9 => &letters[..],
        10 => &letters[1..],
        _ => return None,
    };
    bits.iter()
        .enumerate()
        .try_fold(0, |mode, (index, letter)| {
            let set = match (index % 3, letter) {
                (0, 'r') | (1, 'w') | (2, 'x') => true,
                // Setuid and setgid ("s") and sticky ("t") stand where x does, and x is set too;
                // in capitals x is not.
                (2, 's') if index < 6 => true,
                (2, 't') if index == 8 => true,
                (_, '-' | 'S' | 'T') => false,
                _ => return None,
            };
            Some(mode << 1 | u32::from(set))
        })
}

/// How the confirmation names what is about to be deleted: up to four names in full.
fn names_preview(names: &[String]) -> String {
    if names.len() <= 4 {
        names.join(", ")
    } else {
        format!("{}, and {} more", names[..3].join(", "), names.len() - 3)
    }
}

/// Run blocking file work off the async runtime's worker threads. `failed` is what to say when
/// the work itself broke down.
async fn blocking(
    failed: &'static str,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| failed.to_string())?
}

/// A side as an action on its selection finds it.
struct Target {
    /// The id of the pane.
    id: i32,
    source: Source,
    /// The folder on screen.
    path: String,
    loaded: bool,
    selected: Vec<Entry>,
}

impl Target {
    fn kind(&self) -> Kind {
        self.source.kind()
    }

    /// The list of drives of this PC is on screen.
    fn top(&self) -> bool {
        self.kind() == Kind::Local && self.path.is_empty()
    }

    fn selected(&self) -> Vec<&Entry> {
        self.selected.iter().collect()
    }
}

impl App {
    fn files_target(&self, id: i32, side: usize) -> Option<Target> {
        self.with_files(id, |pane| {
            let state = pane.sides.get(side)?.borrow();
            Some(Target {
                id,
                source: state.source.clone(),
                path: state.path.clone(),
                loaded: state.loaded,
                selected: state
                    .selected
                    .iter()
                    .filter_map(|row| state.entries.get(*row).cloned())
                    .collect(),
            })
        })
        .flatten()
    }

    /// Rename the one selected item of a side to `new_name`, then list the folder again.
    pub(super) fn files_rename(self: &Rc<Self>, id: i32, side: usize, new_name: &str) {
        let Some(target) = self.files_target(id, side) else {
            return;
        };
        let mut problem = rename_problem(
            target.kind(),
            target.loaded,
            target.top(),
            &target.selected(),
        );
        if problem.is_empty() {
            problem = item_name_problem(target.kind(), new_name);
        }
        if !problem.is_empty() {
            self.files_fail(id, side, problem.to_string());
            return;
        }
        let old_name = target.selected[0].name.clone();
        let new_name = new_name.trim().to_string();
        if old_name == new_name {
            return;
        }
        let backend = self.backend.clone();
        let (source, path) = (target.source, target.path);
        let (place, folder) = (source.clone(), path.clone());
        self.run(
            async move {
                match place {
                    Source::Local => {
                        blocking("The item could not be renamed.", move || {
                            rename_local(&folder, &old_name, &new_name)
                        })
                        .await
                    }
                    Source::Host(host) => backend
                        .sftp_rename(host, folder, old_name, new_name)
                        .await
                        .map(|_| ())
                        .map_err(|error| friendly_with(&error, TAKEN)),
                    Source::None | Source::Device(_) => Ok(()),
                }
            },
            move |app, result| app.files_changed(id, side, source, path, result),
        );
    }

    /// Ask before the selected items of a side are deleted.
    pub(super) fn files_delete_ask(&self, id: i32, side: usize) {
        let Some(target) = self.files_target(id, side) else {
            return;
        };
        let problem = delete_problem(
            target.kind(),
            target.loaded,
            target.top(),
            &target.selected(),
        );
        if !problem.is_empty() {
            self.files_fail(id, side, problem.to_string());
            return;
        }
        let names: Vec<String> = target
            .selected
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let (title, label) = match names.as_slice() {
            [name] => (format!("Delete \"{name}\"?"), "Item"),
            _ => (format!("Delete {} items?", names.len()), "Items"),
        };
        let body = if target.selected.iter().any(|entry| entry.is_dir) {
            "This permanently removes the selected items and everything inside any selected folders."
        } else {
            "This permanently removes the selected items. They cannot be recovered from here."
        };
        let facts = vec![
            fact("Place", self.place_name(&target.source), false),
            fact("Folder", target.path.as_str(), true),
            fact(label, names_preview(&names), true),
        ];
        self.ask_with(
            Question {
                title,
                body: body.into(),
                confirm: "Delete".into(),
                danger: true,
            },
            facts,
            "",
            Pending::DeleteFiles {
                pane: target.id,
                side,
                source: target.source,
                dir: target.path,
                names,
            },
        );
    }

    /// Delete the items the confirmation named, then list the folder again. The items are
    /// deleted one after the other; the first that fails stops the rest.
    pub fn files_delete_run(
        self: &Rc<Self>,
        id: i32,
        side: usize,
        source: Source,
        dir: String,
        names: Vec<String>,
    ) {
        let backend = self.backend.clone();
        let (place, folder) = (source.clone(), dir.clone());
        self.run(
            async move {
                match place {
                    Source::Local => {
                        blocking("The selected items could not be deleted.", move || {
                            delete_local(&folder, &names)
                        })
                        .await
                    }
                    Source::Host(host) => {
                        for name in &names {
                            backend
                                .sftp_remove(host.clone(), join(Kind::Host, &folder, name))
                                .await
                                .map_err(|error| friendly(&error))?;
                        }
                        Ok(())
                    }
                    Source::None | Source::Device(_) => Ok(()),
                }
            },
            move |app, result: Result<(), String>| {
                // Some of the items may be gone all the same: list the folder either way.
                let failed = result.err().unwrap_or_default();
                if !app.files_reload_shown(id, side, source, dir, &failed) && !failed.is_empty() {
                    app.files_fail(id, side, failed);
                }
            },
        );
    }

    /// Set the permission bits of the one selected item of a side, which shows a host.
    pub(super) fn files_chmod(self: &Rc<Self>, id: i32, side: usize, mode: i32) {
        let Some(target) = self.files_target(id, side) else {
            return;
        };
        let problem = chmod_problem(target.kind(), target.loaded, &target.selected());
        if !problem.is_empty() {
            self.files_fail(id, side, problem.to_string());
            return;
        }
        let Some(mode) = u32::try_from(mode).ok().filter(|mode| *mode <= 0o7777) else {
            self.files_fail(id, side, "Invalid permission mode.".into());
            return;
        };
        let Source::Host(host) = target.source.clone() else {
            return;
        };
        let item = join(Kind::Host, &target.path, &target.selected[0].name);
        let backend = self.backend.clone();
        self.run(
            async move {
                backend
                    .sftp_chmod(host, item, mode)
                    .await
                    .map_err(|error| friendly(&error))
            },
            move |app, result| app.files_changed(id, side, target.source, target.path, result),
        );
    }

    /// Show or hide dotfiles and the hidden items of Windows on a side.
    pub(super) fn files_toggle_hidden(&self, id: i32, side: usize) {
        self.with_files(id, |pane| {
            let Some(state) = pane.sides.get(side) else {
                return;
            };
            let mut state = state.borrow_mut();
            state.show_hidden = !state.show_hidden;
            state.arrange(true);
        });
        self.files_publish(id);
    }

    /// Select every row a side lists.
    pub(super) fn files_select_all(&self, id: i32, side: usize) {
        self.with_files(id, |pane| {
            let Some(state) = pane.sides.get(side) else {
                return;
            };
            let mut state = state.borrow_mut();
            let all = (0..state.entries.len()).collect();
            state.set_selected(all);
        });
        self.files_publish(id);
    }

    /// Close a side: it offers the places to choose from again.
    pub(super) fn files_close_side(&self, id: i32, side: usize) {
        self.with_files(id, |pane| {
            if let Some(state) = pane.sides.get(side) {
                state.borrow_mut().switch(Source::None);
            }
        });
        self.files_retitle(id);
        self.files_publish(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(name: &str, is_dir: bool, is_symlink: bool, is_drive: bool) -> Entry {
        Entry {
            name: name.into(),
            is_dir,
            is_symlink,
            is_drive,
            is_hidden: name.starts_with('.'),
            size: 0,
            modified: None,
            permissions: String::new(),
        }
    }

    #[test]
    fn parses_posix_permission_strings_into_octal_modes() {
        assert_eq!(parse_permissions("drwxr-xr-x"), Some(0o755));
        assert_eq!(parse_permissions("-rw-r--r--"), Some(0o644));
        assert_eq!(parse_permissions("-rw-------"), Some(0o600));
        assert_eq!(parse_permissions("rwxrwxrwx"), Some(0o777));
        assert_eq!(parse_permissions("----------"), Some(0o000));
        assert_eq!(parse_permissions("-rwsr-xr-t"), Some(0o755));
        assert_eq!(parse_permissions("-rwSr--r-T"), Some(0o644));
        assert_eq!(parse_permissions("-rwSr-xr-T"), Some(0o654));
        assert_eq!(parse_permissions(""), None);
        assert_eq!(parse_permissions("invalid"), None);
        assert_eq!(parse_permissions("-rwxrwxrwt"), Some(0o777));
        assert_eq!(parse_permissions("-rwtr--r--"), None);
        assert_eq!(parse_permissions("-wr-r--r--"), None);
    }

    #[test]
    fn rename_delete_and_chmod_enforce_place_and_selection_rules() {
        let file = sample_entry("notes.txt", false, false, false);
        let dir = sample_entry("docs", true, false, false);
        let drive = sample_entry("C:\\", true, false, true);
        let link = sample_entry("latest", false, true, false);

        assert_eq!(rename_problem(Kind::Host, true, false, &[&file]), "");
        assert_eq!(rename_problem(Kind::Local, true, false, &[&dir]), "");
        assert_ne!(rename_problem(Kind::Host, true, false, &[&file, &dir]), "");
        assert_ne!(rename_problem(Kind::Local, true, true, &[&drive]), "");
        assert_ne!(rename_problem(Kind::Device, true, false, &[&file]), "");

        assert_eq!(delete_problem(Kind::Host, true, false, &[&file, &dir]), "");
        assert_eq!(delete_problem(Kind::Local, true, false, &[&file]), "");
        assert_ne!(delete_problem(Kind::Local, true, false, &[]), "");
        assert_ne!(delete_problem(Kind::Local, true, false, &[&drive]), "");
        assert_ne!(delete_problem(Kind::Device, true, false, &[&file]), "");

        assert_eq!(chmod_problem(Kind::Host, true, &[&file]), "");
        assert_ne!(chmod_problem(Kind::Host, true, &[&link]), "");
        assert_ne!(chmod_problem(Kind::Host, true, &[&file, &dir]), "");
        assert_ne!(chmod_problem(Kind::Local, true, &[&file]), "");
    }

    #[test]
    fn the_confirmation_names_up_to_four_items_and_counts_the_rest() {
        let names =
            |count: usize| -> Vec<String> { (1..=count).map(|n| format!("f{n}")).collect() };
        assert_eq!(names_preview(&names(1)), "f1");
        assert_eq!(names_preview(&names(4)), "f1, f2, f3, f4");
        assert_eq!(names_preview(&names(6)), "f1, f2, f3, and 3 more");
    }
}
