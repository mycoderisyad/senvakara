//! Native file and folder dialogs that do not stop the app, shared with the other modules.

use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::App;

impl App {
    /// Show a native file dialog without stopping the app, then continue on the UI thread.
    pub fn pick_path(
        self: &Rc<Self>,
        dialog: rfd::FileDialog,
        save: bool,
        done: impl FnOnce(&Rc<App>, PathBuf) + Send + 'static,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.dev.picking.replace(true) {
            return;
        }
        // Owned by the main window: the dialog stays in front of it and blocks it meanwhile.
        let dialog = dialog.set_parent(&ui.window().window_handle());
        std::thread::spawn(move || {
            let picked = if save {
                dialog.save_file()
            } else {
                dialog.pick_file()
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    app.dev.picking.set(false);
                    if let Some(path) = picked {
                        done(&app, path);
                    }
                }
            });
        });
    }

    /// Like [`App::pick_path`], for choosing a folder. `done` gets the folder as text.
    pub fn pick_folder(
        self: &Rc<Self>,
        dialog: rfd::FileDialog,
        done: impl FnOnce(&Rc<App>, String) + Send + 'static,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.dev.picking.replace(true) {
            return;
        }
        let dialog = dialog.set_parent(&ui.window().window_handle());
        std::thread::spawn(move || {
            let picked = dialog.pick_folder();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(app) = crate::app() {
                    app.dev.picking.set(false);
                    if let Some(path) = picked {
                        done(&app, path.display().to_string());
                    }
                }
            });
        });
    }
}
