//! Saving the host form: the last check, the host itself, then its secrets, and opening the
//! host when the user pressed Connect.

use std::rc::Rc;

use senvakara_app::{CoreError, HostUpsert, HostView, Protocol};
use slint::{ComponentHandle, SharedString};

use crate::inventory::friendly_with;
use crate::{App, HostForm};

use super::form::show_problems;
use super::secrets::{changes, Change};
use super::validation::{check, Method, Problems};

/// Store or remove the host's secrets, in order. Stops at the first one that fails.
async fn apply(
    backend: &std::sync::Arc<senvakara_app::App>,
    host: HostView,
    secrets: Vec<Change>,
) -> (HostView, Option<CoreError>) {
    let mut host = host;
    for secret in secrets {
        let id = host.id.clone();
        let result = match secret {
            Change::Password(value) => backend.host_set_password(id, value).await,
            Change::Passphrase(value) => backend.host_set_passphrase(id, value).await,
            Change::RdpPassword(value) => backend.host_set_rdp_password(id, value).await,
        };
        match result {
            Ok(view) => host = view,
            Err(error) => return (host, Some(error)),
        }
    }
    (host, None)
}

impl App {
    /// The form check plus what only this computer can tell: whether the key file is there,
    /// and whether the chosen key is still in the Keychain.
    pub(super) fn host_form_check(&self, form: &HostForm) -> Result<HostUpsert, Problems> {
        let fields = self.host_form_fields(form);
        let checked = check(&fields, self.host_form_password(form));
        if fields.protocol != Protocol::Ssh {
            return checked;
        }
        let here = match fields.method {
            Method::KeyFile => {
                let key = fields.key_path.trim();
                (!key.is_empty() && !std::path::Path::new(key).is_file())
                    .then_some("There is no file at this path.")
            }
            Method::Keychain => (!fields.key_id.is_empty()
                && self.keys.key(&fields.key_id).is_none())
            .then_some("This key is no longer in the Keychain. Choose another one."),
            _ => None,
        };
        match here {
            Some(problem) => {
                let mut problems = checked.err().unwrap_or_default();
                problems.key = problem;
                Err(problems)
            }
            None => checked,
        }
    }

    /// Save the form. With `connect`, the host is opened once it is saved.
    pub(super) fn host_form_save(self: &Rc<Self>, connect: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        if form.get_saving() {
            return;
        }
        self.hosts.submitted.set(true);
        form.set_error(SharedString::new());
        // A tag that was typed but not added yet belongs to the host: add it now, or stop and
        // let the field say why it cannot be added.
        if !form.get_tag_draft().trim().is_empty() {
            self.host_form_add_tag();
            if !form.get_tag_error().is_empty() {
                return;
            }
        }
        let input = match self.host_form_check(&form) {
            Ok(input) => {
                show_problems(&form, &Problems::default());
                input
            }
            Err(problems) => {
                show_problems(&form, &problems);
                return;
            }
        };
        let fields = self.host_form_fields(&form);
        let secrets = changes(
            fields.protocol,
            fields.method,
            &self.host_form_typed(&form),
            self.hosts.saved.get(),
        );
        let editing = self.hosts.editing.borrow().clone();
        form.set_saving(true);
        let backend = self.backend.clone();
        self.run(
            async move {
                let saved = match editing {
                    Some(id) => backend.host_update(id, input).await,
                    None => backend.host_create(input),
                }?;
                // (the host as saved, what went wrong with its secrets afterwards)
                Ok(apply(&backend, saved, secrets).await)
            },
            move |app, result: Result<(HostView, Option<CoreError>), CoreError>| {
                app.host_form_saved(result, connect)
            },
        );
    }

    fn host_form_saved(
        self: &Rc<Self>,
        result: Result<(HostView, Option<CoreError>), CoreError>,
        connect: bool,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        form.set_saving(false);
        match result {
            Err(error) => form.set_error(
                friendly_with(&error, "A host with these details already exists.").into(),
            ),
            Ok((host, Some(error))) => {
                // The host exists now: keep the form on it so the secret can be tried again.
                self.reload();
                self.host_form_open(Some(&host.id));
                ui.global::<HostForm>().set_error(
                    format!(
                        "The host was saved, but its password or passphrase was not. {}",
                        friendly_with(&error, "It could not be stored.")
                    )
                    .into(),
                );
            }
            Ok((host, None)) => {
                self.host_form_close();
                self.reload();
                self.select_item("host", &host.id);
                if host.protocol == Protocol::Rdp {
                    if connect {
                        self.host_action(&host.id, "rdp");
                    }
                } else if host.trust_state != "trusted" {
                    // A host is of no use until its key is trusted: ask right away, and go on
                    // to the terminal afterwards when that is what the user asked for.
                    *self.hosts.connect_after_trust.borrow_mut() = connect.then(|| host.id.clone());
                    self.host_action(&host.id, "trust");
                } else if connect {
                    self.host_action(&host.id, "terminal");
                }
            }
        }
    }
}
