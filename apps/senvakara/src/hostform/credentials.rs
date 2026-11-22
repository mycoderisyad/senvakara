//! The parts of the host form that are more than a text field: the tags and the group, the
//! password that is saved or being replaced, and what an SSH host signs in with.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{App, HostForm, KeyChoice};

use super::form::chip_rows;
use super::validation::{tag_problem, Method};

impl App {
    pub(super) fn host_form_show_tags(&self, form: &HostForm) {
        form.set_tags(chip_rows(&self.hosts.tags.borrow(), true));
    }

    /// Take a suggested group.
    pub(super) fn host_form_pick_group(&self, name: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.global::<HostForm>().set_group(name.into());
        self.host_form_edited();
    }

    /// Add the typed tag, or say why it cannot be added.
    pub(super) fn host_form_add_tag(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        let draft = form.get_tag_draft();
        let problem = tag_problem(&draft, &self.hosts.tags.borrow());
        form.set_tag_error(problem.into());
        if !problem.is_empty() {
            return;
        }
        self.hosts.tags.borrow_mut().push(draft.trim().to_string());
        form.set_tag_draft(SharedString::new());
        self.host_form_show_tags(&form);
    }

    pub(super) fn host_form_remove_tag(&self, index: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        {
            let mut tags = self.hosts.tags.borrow_mut();
            if index < 0 || index as usize >= tags.len() {
                return;
            }
            tags.remove(index as usize);
        }
        let form = ui.global::<HostForm>();
        form.set_tag_error(SharedString::new());
        self.host_form_show_tags(&form);
    }

    /// Replace, remove or keep the saved password. Only a host that has one can do that.
    pub(super) fn host_form_set_password_mode(&self, mode: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !self.hosts.saved.get().password || !["saved", "replace", "remove"].contains(&mode) {
            return;
        }
        let form = ui.global::<HostForm>();
        form.set_password_mode(mode.into());
        // A password typed before changing one's mind is not kept around.
        form.set_password(SharedString::new());
        form.set_show_password(false);
        self.host_form_edited();
    }

    /// Show what the host signs in with: the method, and the chip of a Keychain key.
    pub(super) fn host_form_show_method(&self, form: &HostForm, method: Method) {
        form.set_auth(method.as_str().into());
        let key = match method {
            Method::Keychain => self.keys.key(&self.hosts.key_id.borrow()),
            _ => None,
        };
        form.set_key_certificate(key.as_ref().is_some_and(|key| key.certificate.is_some()));
        form.set_key_label(
            key.map(|key| key.label)
                .unwrap_or_else(|| "Key from the Keychain".into())
                .into(),
        );
    }

    /// Sign in with a password, or through the agent.
    pub(super) fn host_form_use_method(&self, method: Method) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        self.hosts.key_id.borrow_mut().clear();
        if method != Method::Password {
            // The password field goes away: do not keep what was typed in it.
            form.set_password(SharedString::new());
        }
        self.host_form_show_method(&form, method);
        self.host_form_edited();
    }

    /// Sign in with this key of the Keychain.
    pub fn host_form_use_key(&self, key_id: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        if !form.get_open() || form.get_protocol() != "ssh" {
            return;
        }
        *self.hosts.key_id.borrow_mut() = key_id.to_string();
        form.set_password(SharedString::new());
        self.host_form_show_method(&form, Method::Keychain);
        self.host_form_edited();
    }

    /// Sign in with a key file on this PC; asks which one when none is named yet.
    pub(super) fn host_form_use_key_file(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let empty = ui.global::<HostForm>().get_key_path().trim().is_empty();
        self.host_form_use_method(Method::KeyFile);
        if empty {
            self.host_form_browse();
        }
    }

    /// The Keychain changed: list its keys in the form's menu and rename the chosen one.
    pub fn host_form_keys_changed(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        let choices: Vec<KeyChoice> = self
            .keys
            .keys()
            .iter()
            .map(|key| KeyChoice {
                id: key.id.as_str().into(),
                label: key.label.as_str().into(),
                detail: format!("Type {}", crate::keychain::type_label(&key.algorithm)).into(),
                certificate: key.certificate.is_some(),
                fido: key.kind == senvakara_app::KeyKind::Fido2,
            })
            .collect();
        form.set_keys(ModelRc::new(VecModel::from(choices)));
        if form.get_open() && form.get_auth() == "keychain" {
            self.host_form_show_method(&form, Method::Keychain);
        }
    }

    pub(super) fn host_form_browse(self: &Rc<Self>) {
        let dialog = crate::keychain::key_file_dialog("Choose the SSH private key");
        self.pick_path(dialog, false, |app, path| {
            let Some(ui) = app.ui.upgrade() else { return };
            let form = ui.global::<HostForm>();
            if form.get_open() {
                form.set_key_path(path.display().to_string().into());
                app.host_form_edited();
            }
        });
    }
}
