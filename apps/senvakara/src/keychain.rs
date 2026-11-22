//! The Keychain section of Home (CRED-02): the SSH keys Senvakara keeps for signing in to
//! hosts, and the form that adds, edits or deletes one. A private key and its passphrase go
//! from the form to Windows Credential Manager and are never read back; the list only holds
//! labels, types, public keys and certificates.

mod form;
mod present;
mod validation;

pub use present::type_label;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use senvakara_app::KeyView;
use slint::{ComponentHandle, VecModel};

use crate::{App, KeyItem, Keychain};

use validation::Kept;

/// What the form is adding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// A private key, with or without a certificate.
    #[default]
    Key,
    /// A private key whose certificate is asked for.
    Certificate,
    /// A FIDO2 security key, known by its public key.
    Fido2,
}

/// Where to go once the form is closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Just close the form.
    Close,
    /// Start a new entry.
    New(Mode),
    /// Open the form for this key.
    Edit(String),
    /// Start a new key from this file.
    Import(PathBuf),
}

#[derive(Default)]
pub struct State {
    /// The keys, as the backend last listed them. No secret is in here.
    keys: RefCell<Vec<KeyView>>,
    model: Rc<VecModel<KeyItem>>,
    /// The key being edited; None while adding one.
    editing: RefCell<Option<String>>,
    mode: Cell<Mode>,
    /// The fields that are not secret, as the form opened.
    initial: RefCell<Kept>,
    /// Save was pressed once: the field notes follow the typing from then on.
    submitted: Cell<bool>,
    /// The new key was asked for from the host form: once saved, that host signs in with it.
    for_host: Cell<bool>,
    /// Counts the file reads started: a result that is not the latest is dropped.
    reads: Cell<u32>,
}

impl State {
    /// Every key, as the backend last listed them.
    pub fn keys(&self) -> Vec<KeyView> {
        self.keys.borrow().clone()
    }

    pub fn key(&self, id: &str) -> Option<KeyView> {
        self.keys.borrow().iter().find(|key| key.id == id).cloned()
    }
}

/// A file dialog that starts in the user's `.ssh` folder, where key files usually are.
pub fn key_file_dialog(title: &str) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Ok(home) = std::env::var("USERPROFILE") {
        let keys = std::path::Path::new(&home).join(".ssh");
        if keys.is_dir() {
            dialog = dialog.set_directory(keys);
        }
    }
    dialog
}

impl App {
    pub fn init_keychain(self: &Rc<Self>, ui: &crate::MainWindow) {
        let view = ui.global::<Keychain>();
        view.set_keys(self.keys.model.clone().into());
        view.on_new_key({
            let app = self.clone();
            move |kind| {
                let mode = match kind.as_str() {
                    "certificate" => Mode::Certificate,
                    "fido2" => Mode::Fido2,
                    _ => Mode::Key,
                };
                app.keychain_go(Next::New(mode));
            }
        });
        view.on_import_file({
            let app = self.clone();
            move || app.keychain_browse()
        });
        view.on_select({
            let app = self.clone();
            move |id| app.keychain_go(Next::Edit(id.to_string()))
        });
        view.on_edited({
            let app = self.clone();
            move || app.keychain_edited()
        });
        view.on_set_replace_private({
            let app = self.clone();
            move |on| app.keychain_set_replace(on)
        });
        view.on_save({
            let app = self.clone();
            move || app.keychain_save()
        });
        view.on_cancel({
            let app = self.clone();
            move || app.keychain_go(Next::Close)
        });
        view.on_delete({
            let app = self.clone();
            move || app.keychain_delete_ask()
        });
        self.keychain_reload();
    }
}
