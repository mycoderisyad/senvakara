//! The add/edit host form: opening it, following the typing, saving, and leaving it.

use std::rc::Rc;

use senvakara_app::{CoreError, HostUpsert, HostView};
use slint::{ComponentHandle, SharedString};

use crate::inventory::friendly_with;
use crate::{App, HostForm, Pending, Question};

use super::validation::{check, Fields, Problems};
use super::Next;

fn fields_of(form: &HostForm) -> Fields {
    Fields {
        name: form.get_name().into(),
        address: form.get_address().into(),
        port: form.get_port().into(),
        username: form.get_username().into(),
        os: form.get_os().into(),
        key_path: form.get_key_path().into(),
    }
}

fn show_problems(form: &HostForm, problems: &Problems) {
    form.set_name_error(problems.name.into());
    form.set_address_error(problems.address.into());
    form.set_port_error(problems.port.into());
    form.set_username_error(problems.username.into());
    form.set_key_error(problems.key.into());
}

/// The form check plus what only this computer can tell: whether the key file is there.
fn check_here(fields: &Fields) -> Result<HostUpsert, Problems> {
    let checked = check(fields);
    let key = fields.key_path.trim();
    if !key.is_empty() && !std::path::Path::new(key).is_file() {
        let mut problems = checked.err().unwrap_or_default();
        problems.key = "There is no file at this path.";
        return Err(problems);
    }
    checked
}

impl App {
    /// Whether the user changed anything since the form opened.
    fn host_form_dirty(&self) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        let form = ui.global::<HostForm>();
        form.get_open()
            && (fields_of(&form) != *self.hosts.initial.borrow()
                || !form.get_passphrase().is_empty()
                || form.get_remove_passphrase())
    }

    /// Something else wants the inspector. Returns true when the form is not in the way (it is
    /// closed here if it was open and untouched); otherwise asks, and `next` happens only if
    /// the user agrees to lose what they typed.
    pub fn host_form_leave(self: &Rc<Self>, next: Next) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return true;
        };
        let form = ui.global::<HostForm>();
        if !form.get_open() {
            return true;
        }
        if form.get_saving() {
            return false;
        }
        if !self.host_form_dirty() {
            form.set_open(false);
            return true;
        }
        let adding = self.hosts.editing.borrow().is_none();
        self.ask(
            Question {
                title: "Discard what you typed?".into(),
                body: if adding {
                    "The new host has not been saved."
                } else {
                    "The changes to this host have not been saved."
                }
                .into(),
                confirm: "Discard".into(),
                danger: true,
            },
            Pending::DiscardHostForm(next),
        );
        false
    }

    /// The user agreed to lose the form.
    pub fn host_form_discard(self: &Rc<Self>, next: Next) {
        self.host_form_close();
        match next {
            Next::Close => {}
            Next::Select(kind, id) => self.select_item(&kind, &id),
            Next::Open(host) => self.host_form_open(host.as_deref()),
        }
    }

    pub(super) fn host_form_close(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let form = ui.global::<HostForm>();
            form.set_open(false);
            // Do not keep a passphrase in a hidden field.
            form.set_passphrase(SharedString::new());
        }
    }

    /// Show the form: filled in for `host_id`, empty for a new host.
    pub fn host_form_open(&self, host_id: Option<&str>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let host = host_id.and_then(|id| self.inv.host(id));
        let ssh = host.as_ref().and_then(|host| host.ssh.as_ref());
        let fields = Fields {
            name: host
                .as_ref()
                .map(|host| host.display_name.clone())
                .unwrap_or_default(),
            address: ssh.map(|ssh| ssh.address.clone()).unwrap_or_default(),
            port: ssh.map(|ssh| ssh.port).unwrap_or(22).to_string(),
            username: ssh.map(|ssh| ssh.username.clone()).unwrap_or_default(),
            os: match host.as_ref().map(|host| host.os_family.as_str()) {
                Some("linux") | None => "linux",
                _ => "unknown",
            }
            .to_string(),
            key_path: ssh
                .map(|ssh| ssh.private_key_path.clone())
                .unwrap_or_default(),
        };
        let form = ui.global::<HostForm>();
        form.set_title(
            match &host {
                Some(host) => format!("Edit {}", host.display_name),
                None => "New host".to_string(),
            }
            .into(),
        );
        form.set_editing(host.is_some());
        form.set_name(fields.name.as_str().into());
        form.set_address(fields.address.as_str().into());
        form.set_port(fields.port.as_str().into());
        form.set_username(fields.username.as_str().into());
        form.set_os(fields.os.as_str().into());
        form.set_key_path(fields.key_path.as_str().into());
        form.set_passphrase(SharedString::new());
        form.set_has_passphrase(ssh.is_some_and(|ssh| ssh.has_passphrase));
        form.set_remove_passphrase(false);
        form.set_error(SharedString::new());
        form.set_saving(false);
        show_problems(&form, &Problems::default());
        *self.hosts.editing.borrow_mut() = host.map(|host| host.id);
        *self.hosts.initial.borrow_mut() = fields;
        self.hosts.submitted.set(false);
        form.set_open(true);
        // A new form starts with the cursor in its first field.
        form.set_revision(form.get_revision().wrapping_add(1));
    }

    pub(super) fn host_form_edited(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        // A passphrase typed after ticking "remove" means: replace it.
        if !form.get_passphrase().is_empty() && form.get_remove_passphrase() {
            form.set_remove_passphrase(false);
        }
        if self.hosts.submitted.get() {
            let problems = check_here(&fields_of(&form)).err().unwrap_or_default();
            show_problems(&form, &problems);
        }
    }

    pub(super) fn host_form_browse(self: &Rc<Self>) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose the SSH private key");
        if let Ok(home) = std::env::var("USERPROFILE") {
            let keys = std::path::Path::new(&home).join(".ssh");
            if keys.is_dir() {
                dialog = dialog.set_directory(keys);
            }
        }
        self.pick_path(dialog, false, |app, path| {
            let Some(ui) = app.ui.upgrade() else { return };
            let form = ui.global::<HostForm>();
            if form.get_open() {
                form.set_key_path(path.display().to_string().into());
                app.host_form_edited();
            }
        });
    }

    pub(super) fn host_form_save(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        if form.get_saving() {
            return;
        }
        self.hosts.submitted.set(true);
        form.set_error(SharedString::new());
        let input = match check_here(&fields_of(&form)) {
            Ok(input) => {
                show_problems(&form, &Problems::default());
                input
            }
            Err(problems) => {
                show_problems(&form, &problems);
                return;
            }
        };
        let editing = self.hosts.editing.borrow().clone();
        let passphrase = form.get_passphrase().to_string();
        let remove = form.get_remove_passphrase();
        form.set_saving(true);
        let backend = self.backend.clone();
        self.run(
            async move {
                // (the host as saved, what went wrong with the passphrase afterwards)
                let saved = match editing {
                    Some(id) => backend.host_update(id, input).await,
                    None => backend.host_create(input),
                }?;
                let secret = if !passphrase.is_empty() {
                    Some(Some(passphrase))
                } else if remove {
                    Some(None)
                } else {
                    None
                };
                let Some(secret) = secret else {
                    return Ok((saved, None));
                };
                match backend.host_set_passphrase(saved.id.clone(), secret).await {
                    Ok(view) => Ok((view, None)),
                    Err(error) => Ok((saved, Some(error))),
                }
            },
            |app, result: Result<(HostView, Option<CoreError>), CoreError>| {
                app.host_form_saved(result)
            },
        );
    }

    fn host_form_saved(self: &Rc<Self>, result: Result<(HostView, Option<CoreError>), CoreError>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        form.set_saving(false);
        match result {
            Err(error) => form.set_error(
                friendly_with(&error, "A host with these details already exists.").into(),
            ),
            Ok((host, Some(error))) => {
                // The host exists now: keep the form on it so the passphrase can be tried again.
                self.reload();
                self.host_form_open(Some(&host.id));
                ui.global::<HostForm>().set_error(
                    format!(
                        "The host was saved, but the passphrase was not. {}",
                        friendly_with(&error, "The passphrase could not be stored.")
                    )
                    .into(),
                );
            }
            Ok((host, None)) => {
                self.host_form_close();
                self.reload();
                self.select_item("host", &host.id);
                // A host is of no use until its key is trusted: ask right away.
                if host.trust_state != "trusted" {
                    self.host_action(&host.id, "trust");
                }
            }
        }
    }
}
