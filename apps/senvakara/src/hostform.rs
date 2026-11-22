//! Adding, editing and removing a remote host, and the addresses it can be reached at. The
//! form lives in the inspector; nothing is saved until Save, and a form the user has typed in
//! is never dropped without asking.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::ComponentHandle;

use crate::{App, HostForm, Inventory, NetPaths};

mod form;
mod paths;
mod removal;
mod validation;

use validation::Fields;

/// Where to go once the form is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Just close the form.
    Close,
    /// Select something on Home (kind, id).
    Select(String, String),
    /// Open the form again: for this host, or empty for a new one.
    Open(Option<String>),
}

#[derive(Default)]
pub struct State {
    /// The host being edited; None while adding one.
    editing: RefCell<Option<String>>,
    /// The fields as the form opened: anything else means the user typed something.
    initial: RefCell<Fields>,
    /// Save was pressed once: the field notes follow the typing from then on.
    submitted: Cell<bool>,
    /// The host whose network paths the inspector shows.
    paths_host: RefCell<String>,
}

impl App {
    pub fn init_host_form(self: &Rc<Self>, ui: &crate::MainWindow) {
        ui.global::<Inventory>().on_add_host({
            let app = self.clone();
            move || {
                if app.host_form_leave(Next::Open(None)) {
                    app.host_form_open(None);
                }
            }
        });
        let form = ui.global::<HostForm>();
        form.on_edited({
            let app = self.clone();
            move || app.host_form_edited()
        });
        form.on_set_os({
            let app = self.clone();
            move |os| {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<HostForm>().set_os(os);
                }
            }
        });
        form.on_set_remove_passphrase({
            let app = self.clone();
            move |on| {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<HostForm>().set_remove_passphrase(on);
                }
            }
        });
        form.on_browse({
            let app = self.clone();
            move || app.host_form_browse()
        });
        form.on_save({
            let app = self.clone();
            move || app.host_form_save()
        });
        form.on_cancel({
            let app = self.clone();
            move || {
                if app.host_form_leave(Next::Close) {
                    app.host_form_close();
                }
            }
        });

        let paths = ui.global::<NetPaths>();
        paths.on_edited({
            let app = self.clone();
            move || app.paths_validate()
        });
        paths.on_add({
            let app = self.clone();
            move || app.paths_add(None)
        });
        paths.on_add_hint({
            let app = self.clone();
            move |address| app.paths_add(Some(address.to_string()))
        });
        paths.on_remove({
            let app = self.clone();
            move |endpoint| app.paths_remove_ask(&endpoint)
        });
    }
}
