//! The key form: listing the keys, opening the form, importing a key file into it, saving,
//! and deleting a key after asking.

use std::path::PathBuf;
use std::rc::Rc;

use senvakara_app::{CoreError, KeyCreate, KeyImport, KeyKind, KeyUpdate, KeyView};
use slint::{ComponentHandle, SharedString};

use crate::inventory::{fact, friendly, friendly_with};
use crate::{App, Keychain, Pending, Question};

use super::present::{key_item, type_label};
use super::validation::{check_edit, check_new, Kept, Problems};
use super::{key_file_dialog, Mode, Next};

fn kept_of(view: &Keychain) -> Kept {
    Kept {
        label: view.get_label().into(),
        public_key: view.get_public_key().into(),
        certificate: view.get_certificate().into(),
    }
}

fn show_problems(view: &Keychain, problems: &Problems) {
    view.set_label_error(problems.label.into());
    view.set_private_error(problems.private_key.into());
    view.set_public_error(problems.public_key.into());
    view.set_certificate_error(problems.certificate.into());
    view.set_passphrase_error(problems.passphrase.into());
}

/// Text that is set, or nothing when the field is empty.
fn filled(text: SharedString) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_string())
}

impl App {
    /// Read the keys again and redraw: the Keychain page and the host form's key menu.
    pub fn keychain_reload(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        match self.backend.keychain_list() {
            Ok(keys) => {
                self.keys
                    .model
                    .set_vec(keys.iter().map(key_item).collect::<Vec<_>>());
                *self.keys.keys.borrow_mut() = keys;
                view.set_error(SharedString::new());
            }
            Err(error) => view.set_error(friendly(&error).into()),
        }
        self.host_form_keys_changed();
    }

    /// Whether the user changed anything since the form opened.
    fn keychain_dirty(&self, view: &Keychain) -> bool {
        view.get_form_open()
            && (kept_of(view) != *self.keys.initial.borrow()
                || !view.get_private_key().is_empty()
                || !view.get_passphrase().is_empty())
    }

    /// Go somewhere else with the form: straight away when nothing would be lost, otherwise
    /// after asking.
    pub(super) fn keychain_go(self: &Rc<Self>, next: Next) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        if view.get_saving() {
            return;
        }
        // A file dropped on the new-key form fills that form in: nothing typed is at stake
        // that the user did not just choose to replace.
        let filling_in = matches!(next, Next::Import(_))
            && view.get_form_open()
            && self.keys.editing.borrow().is_none()
            && self.keys.mode.get() != Mode::Fido2;
        if filling_in || !self.keychain_dirty(&view) {
            self.keychain_discard(next);
            return;
        }
        let adding = self.keys.editing.borrow().is_none();
        self.ask(
            Question {
                title: "Discard what you typed?".into(),
                body: if adding {
                    "The new key has not been saved."
                } else {
                    "The changes to this key have not been saved."
                }
                .into(),
                confirm: "Discard".into(),
                danger: true,
            },
            Pending::DiscardKeyForm(next),
        );
    }

    /// Leave the form as it is and do `next`. Also what runs once the user agreed to lose it.
    pub fn keychain_discard(self: &Rc<Self>, next: Next) {
        match next {
            Next::Close => self.keychain_close(),
            Next::New(mode) => self.keychain_open(mode, None),
            Next::Edit(id) => match self.keys.key(&id) {
                Some(key) => {
                    let mode = match key.kind {
                        KeyKind::Key => Mode::Key,
                        KeyKind::Fido2 => Mode::Fido2,
                    };
                    self.keychain_open(mode, Some(&key));
                }
                None => self.keychain_close(),
            },
            Next::Import(path) => {
                let filling_in = self.ui.upgrade().is_some_and(|ui| {
                    ui.global::<Keychain>().get_form_open()
                        && self.keys.editing.borrow().is_none()
                        && self.keys.mode.get() != Mode::Fido2
                });
                if !filling_in {
                    self.keychain_open(Mode::Key, None);
                }
                self.keychain_import(path);
            }
        }
    }

    fn keychain_close(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        view.set_form_open(false);
        // Do not keep a secret in a hidden field.
        view.set_private_key(SharedString::new());
        view.set_passphrase(SharedString::new());
        view.set_drop_hover(false);
        self.keys.for_host.set(false);
        // A file still being read has nowhere to go any more.
        self.keys.reads.set(self.keys.reads.get().wrapping_add(1));
    }

    /// Show the form: for `key`, or empty for a new entry of `mode`.
    fn keychain_open(&self, mode: Mode, key: Option<&KeyView>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        let kept = Kept {
            label: key.map(|key| key.label.clone()).unwrap_or_default(),
            public_key: key.map(|key| key.public_key.clone()).unwrap_or_default(),
            certificate: key
                .and_then(|key| key.certificate.clone())
                .unwrap_or_default(),
        };
        view.set_form_title(
            match (key, mode) {
                (Some(key), _) => format!("Edit {}", key.label),
                (None, Mode::Key) => "New Key".to_string(),
                (None, Mode::Certificate) => "New Key with Certificate".to_string(),
                (None, Mode::Fido2) => "New FIDO2 Key".to_string(),
            }
            .into(),
        );
        view.set_form_kind(if mode == Mode::Fido2 { "fido2" } else { "key" }.into());
        view.set_editing_id(key.map(|key| key.id.as_str()).unwrap_or("").into());
        view.set_label(kept.label.as_str().into());
        view.set_public_key(kept.public_key.as_str().into());
        view.set_certificate(kept.certificate.as_str().into());
        view.set_private_key(SharedString::new());
        view.set_passphrase(SharedString::new());
        view.set_private_saved(key.is_some_and(|key| key.has_private_key));
        view.set_replace_private(false);
        view.set_has_passphrase(key.is_some_and(|key| key.has_passphrase));
        view.set_type_text(
            key.map(|key| type_label(&key.algorithm))
                .unwrap_or_default()
                .into(),
        );
        view.set_used_by(
            key.map(|key| key.used_by.join(", "))
                .unwrap_or_default()
                .into(),
        );
        view.set_form_error(SharedString::new());
        view.set_saving(false);
        show_problems(&view, &Problems::default());
        *self.keys.editing.borrow_mut() = key.map(|key| key.id.clone());
        self.keys.mode.set(mode);
        self.keys.for_host.set(false);
        *self.keys.initial.borrow_mut() = kept;
        self.keys.submitted.set(false);
        self.keys.reads.set(self.keys.reads.get().wrapping_add(1));
        view.set_form_open(true);
        // A new form starts with the cursor in its first field.
        view.set_revision(view.get_revision().wrapping_add(1));
    }

    /// The host form asked for a new key: show the Keychain with an empty key form. Once the
    /// key is saved, the host form signs in with it.
    pub fn keychain_new_for_host(self: &Rc<Self>) {
        self.go_to_section("keychain");
        self.keychain_go(Next::New(Mode::Key));
        // Set after the form opened: opening or closing a form forgets an earlier wish.
        let opened = self.ui.upgrade().is_some_and(|ui| {
            ui.global::<Keychain>().get_form_open() && self.keys.editing.borrow().is_none()
        });
        self.keys.for_host.set(opened);
    }

    fn keychain_problems(&self, view: &Keychain) -> Problems {
        let kept = kept_of(view);
        match self.keys.editing.borrow().as_deref() {
            None => check_new(
                self.keys.mode.get(),
                &view.get_private_key(),
                !view.get_passphrase().is_empty(),
                &kept,
            ),
            Some(_) => {
                let new_private = view.get_replace_private().then(|| view.get_private_key());
                check_edit(
                    self.keys.mode.get() == Mode::Fido2,
                    new_private.as_deref(),
                    !view.get_passphrase().is_empty(),
                    &kept,
                )
            }
        }
    }

    pub(super) fn keychain_edited(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        let problems = self.keychain_problems(&view);
        if self.keys.submitted.get() {
            show_problems(&view, &problems);
        } else {
            // Which key is encrypted is told as soon as it is pasted or imported.
            view.set_passphrase_error(problems.passphrase.into());
        }
    }

    /// Show or hide the field that replaces the saved private key.
    pub(super) fn keychain_set_replace(&self, on: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        view.set_replace_private(on);
        if !on {
            view.set_private_key(SharedString::new());
        }
        self.keychain_edited();
    }

    /// Ask for a key file and start a new key from it.
    pub(super) fn keychain_browse(self: &Rc<Self>) {
        let dialog = key_file_dialog("Choose the private key file to import");
        self.pick_path(dialog, false, |app, path| {
            app.keychain_go(Next::Import(path))
        });
    }

    /// Files from Explorer were dropped while the Keychain is showing: the first one is
    /// imported as a new key.
    pub fn keychain_files_dropped(self: &Rc<Self>, mut paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.keychain_go(Next::Import(paths.swap_remove(0)));
    }

    /// Read a key file into the new-key form. Nothing is stored until Save.
    fn keychain_import(self: &Rc<Self>, path: PathBuf) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        view.set_form_error(SharedString::new());
        let asked = self.keys.reads.get().wrapping_add(1);
        self.keys.reads.set(asked);
        let backend = self.backend.clone();
        self.run(
            async move { backend.keychain_import_file(path).await },
            move |app, result: Result<KeyImport, CoreError>| {
                // The form was closed or reopened meanwhile: the file is not wanted any more.
                if app.keys.reads.get() != asked {
                    return;
                }
                let Some(ui) = app.ui.upgrade() else { return };
                let view = ui.global::<Keychain>();
                match result {
                    Ok(file) => {
                        if view.get_label().trim().is_empty() {
                            view.set_label(file.label.into());
                        }
                        view.set_private_key(file.private_key.into());
                        view.set_public_key(file.public_key.unwrap_or_default().trim().into());
                        view.set_certificate(file.certificate.unwrap_or_default().trim().into());
                        app.keychain_edited();
                    }
                    Err(error) => view.set_form_error(
                        format!(
                            "The file could not be imported. {}",
                            friendly_with(&error, "It could not be read.")
                        )
                        .into(),
                    ),
                }
            },
        );
    }

    pub(super) fn keychain_save(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        if view.get_saving() {
            return;
        }
        self.keys.submitted.set(true);
        view.set_form_error(SharedString::new());
        let problems = self.keychain_problems(&view);
        show_problems(&view, &problems);
        if problems.any() {
            return;
        }
        let editing = self.keys.editing.borrow().clone();
        let fido = self.keys.mode.get() == Mode::Fido2;
        let label = view.get_label().trim().to_string();
        let certificate = filled(view.get_certificate()).map(|text| text.trim().to_string());
        let passphrase = Some(view.get_passphrase().to_string()).filter(|text| !text.is_empty());
        let private_key = view.get_private_key().to_string();
        let public_key = filled(view.get_public_key()).map(|text| text.trim().to_string());
        let replace = view.get_replace_private();
        view.set_saving(true);
        let backend = self.backend.clone();
        self.run(
            async move {
                match editing {
                    Some(id) => {
                        let input = KeyUpdate {
                            label,
                            certificate,
                            private_key: (replace && !fido).then_some(private_key),
                            passphrase,
                        };
                        backend.keychain_update(id, input).await
                    }
                    None => {
                        let input = KeyCreate {
                            label,
                            kind: if fido { KeyKind::Fido2 } else { KeyKind::Key },
                            private_key: if fido { String::new() } else { private_key },
                            passphrase,
                            public_key,
                            certificate,
                        };
                        backend.keychain_create(input).await
                    }
                }
            },
            |app, result| app.keychain_saved(result),
        );
    }

    fn keychain_saved(self: &Rc<Self>, result: Result<KeyView, CoreError>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Keychain>();
        view.set_saving(false);
        match result {
            Err(error) => view.set_form_error(
                friendly_with(&error, "A key with this label already exists.").into(),
            ),
            Ok(key) => {
                let for_host = self.keys.for_host.get();
                self.keychain_close();
                self.keychain_reload();
                if for_host {
                    // Back to the host that asked for the key; it signs in with it now.
                    self.host_form_use_key(&key.id);
                    self.go_to_section("hosts");
                }
            }
        }
    }

    pub(super) fn keychain_delete_ask(self: &Rc<Self>) {
        let Some(id) = self.keys.editing.borrow().clone() else {
            return;
        };
        let Some(key) = self.keys.key(&id) else {
            return;
        };
        let mut facts = vec![
            fact("Key", key.label.as_str(), false),
            fact("Type", type_label(&key.algorithm), false),
        ];
        if !key.used_by.is_empty() {
            facts.push(fact("Used by", key.used_by.join(", "), false));
        }
        self.ask_with(
            Question {
                title: format!("Delete {}?", key.label),
                body: "The key is removed from Senvakara and its private key from Windows Credential Manager. Key files on this PC and the public key on your hosts are not touched.".into(),
                confirm: "Delete key".into(),
                danger: true,
            },
            facts,
            if key.used_by.is_empty() {
                ""
            } else {
                "A key that a host still signs in with cannot be deleted. Choose another sign-in method for those hosts first."
            },
            Pending::DeleteKey(id),
        );
    }

    pub fn keychain_delete(self: &Rc<Self>, id: String) {
        let backend = self.backend.clone();
        let asked = id.clone();
        self.run(
            async move { backend.keychain_delete(id).await },
            move |app, result| {
                match result {
                    Ok(()) => {
                        if app.keys.editing.borrow().as_deref() == Some(asked.as_str()) {
                            app.keychain_close();
                        }
                    }
                    Err(error) => {
                        if let Some(ui) = app.ui.upgrade() {
                            ui.global::<Keychain>().set_form_error(
                                format!(
                                    "The key was not deleted. {}",
                                    friendly_with(
                                        &error,
                                        "A host still signs in with it. Choose another sign-in method for that host first."
                                    )
                                )
                                .into(),
                            );
                        }
                    }
                }
                app.keychain_reload();
            },
        );
    }
}
