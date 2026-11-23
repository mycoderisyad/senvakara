//! Adding, editing and removing a remote host (SSH or Remote Desktop), and the addresses it
//! can be reached at. The form lives in the inspector; nothing is saved until Save or Connect,
//! and a form the user has typed in is never dropped without asking. A password or passphrase
//! goes from its field to Windows Credential Manager and is never read back into the form.

mod chips;
mod credentials;
mod form;
mod keyfile;
mod paths;
mod removal;
mod save;
mod secrets;
mod validation;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::ComponentHandle;

use crate::{App, HostForm, Inventory, NetPaths};

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
    /// The tags of the form, in the order they were added.
    tags: RefCell<Vec<String>>,
    /// The Keychain key the form signs in with, when that is the chosen method.
    key_id: RefCell<String>,
    /// The key file the form signs in with, and whether it asks for a passphrase (None until
    /// the backend has read it, or when it could not).
    key_file: RefCell<(String, Option<bool>)>,
    /// Which secrets the host had when the form opened.
    saved: Cell<secrets::Saved>,
    /// A host saved with Connect whose key must be trusted first: its terminal opens as soon
    /// as the user accepts the key.
    connect_after_trust: RefCell<Option<String>>,
}

impl State {
    /// Whether accepting the key of `host_id` should go on to open its terminal. Asking
    /// uses the wish up.
    pub fn take_connect_after_trust(&self, host_id: &str) -> bool {
        let mut wanted = self.connect_after_trust.borrow_mut();
        let wanted_here = wanted.as_deref() == Some(host_id);
        *wanted = None;
        wanted_here
    }
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
        form.on_set_protocol({
            let app = self.clone();
            move |protocol| app.host_form_set_protocol(&protocol)
        });
        form.on_set_os({
            let app = self.clone();
            move |os| {
                if let Some(ui) = app.ui.upgrade() {
                    ui.global::<HostForm>().set_os(os);
                }
            }
        });
        form.on_pick_group({
            let app = self.clone();
            move |name| app.host_form_pick_group(&name)
        });
        form.on_add_tag({
            let app = self.clone();
            move || app.host_form_add_tag()
        });
        form.on_remove_tag({
            let app = self.clone();
            move |index| app.host_form_remove_tag(index)
        });
        form.on_toggle_password({
            let app = self.clone();
            move || {
                if let Some(ui) = app.ui.upgrade() {
                    let form = ui.global::<HostForm>();
                    form.set_show_password(!form.get_show_password());
                }
            }
        });
        form.on_set_password_mode({
            let app = self.clone();
            move |mode| app.host_form_set_password_mode(&mode)
        });
        form.on_pick_key({
            let app = self.clone();
            move |id| app.host_form_use_key(&id)
        });
        form.on_pick_key_file({
            let app = self.clone();
            move || app.host_form_use_key_file()
        });
        form.on_pick_agent({
            let app = self.clone();
            move || app.host_form_use_method(validation::Method::Agent)
        });
        form.on_clear_key({
            let app = self.clone();
            move || app.host_form_use_method(validation::Method::Password)
        });
        form.on_new_key({
            let app = self.clone();
            move || app.keychain_new_for_host()
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
            move || app.host_form_save(false)
        });
        form.on_connect({
            let app = self.clone();
            move || app.host_form_save(true)
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
