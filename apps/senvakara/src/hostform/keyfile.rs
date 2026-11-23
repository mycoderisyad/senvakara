//! Whether the key file a host signs in with asks for a passphrase (CRED-02). The file is read
//! by the backend, off the UI thread, when the path changes; only the yes or no comes back, so
//! the key text never reaches the form. Until the answer is known nothing is claimed.

use slint::ComponentHandle;

use crate::{App, HostForm};

use super::validation::{key_passphrase_problem, Method};

impl App {
    /// The key file the form signs in with, or "" when it signs in another way.
    fn host_form_key_path(form: &HostForm) -> String {
        if form.get_auth() == Method::KeyFile.as_str() {
            form.get_key_path().trim().to_string()
        } else {
            String::new()
        }
    }

    /// Why the form cannot be saved for want of a passphrase, or "" when it can.
    pub(super) fn host_form_passphrase_problem(&self, form: &HostForm) -> &'static str {
        let path = Self::host_form_key_path(form);
        let encrypted = {
            let known = self.hosts.key_file.borrow();
            (!path.is_empty() && known.0 == path)
                .then_some(known.1)
                .flatten()
        };
        key_passphrase_problem(
            encrypted,
            !form.get_passphrase().is_empty(),
            form.get_has_passphrase(),
            form.get_remove_passphrase(),
        )
    }

    /// Show or clear the note under the passphrase field.
    pub(super) fn host_form_show_passphrase_note(&self, form: &HostForm) {
        form.set_passphrase_error(self.host_form_passphrase_problem(form).into());
    }

    /// The form changed: when it names another key file than the one known, read that one.
    pub(super) fn host_form_follow_key_file(&self, form: &HostForm) {
        let path = Self::host_form_key_path(form);
        let changed = {
            let mut known = self.hosts.key_file.borrow_mut();
            let changed = known.0 != path;
            if changed {
                *known = (path.clone(), None);
            }
            changed
        };
        self.host_form_show_passphrase_note(form);
        if !changed || path.is_empty() {
            return;
        }
        let backend = self.backend.clone();
        self.backend.rt.spawn(async move {
            let asked = path.clone();
            let answer = backend
                .key_file_is_encrypted(std::path::PathBuf::from(asked))
                .await
                .ok();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = crate::app() else { return };
                // The user named another file meanwhile: this answer is not wanted.
                if app.hosts.key_file.borrow().0 != path {
                    return;
                }
                app.hosts.key_file.borrow_mut().1 = answer;
                if let Some(ui) = app.ui.upgrade() {
                    app.host_form_show_passphrase_note(&ui.global::<HostForm>());
                }
            });
        });
    }
}
