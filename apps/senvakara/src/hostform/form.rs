//! The add/edit host form: opening it, following the typing, and leaving it.

use std::rc::Rc;

use senvakara_app::{HostView, Protocol, SshAuth};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{App, ChipItem, ChipRow, HostForm, Pending, Question};

use super::chips;
use super::secrets::{password_state, Saved, Typed};
use super::validation::{rdp_password_lost, Fields, Method, Password, Problems};
use super::Next;

/// The note under the password of a Remote Desktop host whose saved one no longer fits.
const RDP_PASSWORD_MOVED: &str = "The saved password belongs to the old address or user name. Enter it again, or leave it empty and Remote Desktop asks for it.";

/// Labels as chip lines for the view.
pub(super) fn chip_rows(labels: &[String], removable: bool) -> ModelRc<ChipRow> {
    let rows: Vec<ChipRow> = chips::lines(labels, removable)
        .into_iter()
        .map(|line| {
            let line: Vec<ChipItem> = line
                .into_iter()
                .map(|index| ChipItem {
                    text: labels[index].as_str().into(),
                    index: index as i32,
                })
                .collect();
            ChipRow {
                chips: ModelRc::new(VecModel::from(line)),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

fn method_of(name: &str) -> Method {
    match name {
        "key_file" => Method::KeyFile,
        "keychain" => Method::Keychain,
        "agent" => Method::Agent,
        _ => Method::Password,
    }
}

/// The fields of a saved host, as the form shows them.
fn fields_from(host: &HostView) -> Fields {
    let mut fields = Fields {
        protocol: host.protocol,
        name: host.display_name.clone(),
        os: match host.os_family.as_str() {
            "linux" => "linux",
            _ => "unknown",
        }
        .to_string(),
        group: host.group_name.clone().unwrap_or_default(),
        tags: host.tags.clone(),
        ..Fields::default()
    };
    if let Some(rdp) = &host.rdp {
        fields.address = rdp.address.clone();
        fields.port = rdp.port.to_string();
        fields.username = rdp.username.clone();
    }
    if let Some(ssh) = &host.ssh {
        fields.address = ssh.address.clone();
        fields.port = ssh.port.to_string();
        fields.username = ssh.username.clone();
        fields.snippet = ssh.startup_snippet.clone().unwrap_or_default();
        match &ssh.auth {
            SshAuth::KeyFile { path } => {
                fields.method = Method::KeyFile;
                fields.key_path = path.clone();
            }
            SshAuth::Keychain { key_id } => {
                fields.method = Method::Keychain;
                fields.key_id = key_id.clone();
            }
            SshAuth::Password => fields.method = Method::Password,
            SshAuth::Agent => fields.method = Method::Agent,
        }
    }
    fields
}

pub(super) fn show_problems(form: &HostForm, problems: &Problems) {
    form.set_address_error(problems.address.into());
    form.set_group_error(problems.group.into());
    form.set_port_error(problems.port.into());
    form.set_username_error(problems.username.into());
    form.set_password_error(problems.password.into());
    form.set_key_error(problems.key.into());
    form.set_snippet_error(problems.snippet.into());
}

impl App {
    /// What the form holds right now, without its secrets.
    pub(super) fn host_form_fields(&self, form: &HostForm) -> Fields {
        Fields {
            protocol: if form.get_protocol() == "rdp" {
                Protocol::Rdp
            } else {
                Protocol::Ssh
            },
            name: form.get_name().into(),
            address: form.get_address().into(),
            port: form.get_port().into(),
            username: form.get_username().into(),
            os: form.get_os().into(),
            group: form.get_group().into(),
            tags: self.hosts.tags.borrow().clone(),
            method: method_of(&form.get_auth()),
            key_path: form.get_key_path().into(),
            key_id: self.hosts.key_id.borrow().clone(),
            snippet: form.get_snippet().into(),
        }
    }

    /// What the secret fields hold right now.
    pub(super) fn host_form_typed(&self, form: &HostForm) -> Typed {
        Typed {
            password_mode: form.get_password_mode().into(),
            password: form.get_password().into(),
            passphrase: form.get_passphrase().into(),
            remove_passphrase: form.get_remove_passphrase(),
        }
    }

    /// Whether the host will have a password once the form is saved.
    pub(super) fn host_form_password(&self, form: &HostForm) -> Password {
        password_state(&form.get_password_mode(), &form.get_password())
    }

    /// Whether the user changed anything since the form opened.
    fn host_form_dirty(&self) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        let form = ui.global::<HostForm>();
        if !form.get_open() {
            return false;
        }
        let typed = self.host_form_typed(&form);
        self.host_form_fields(&form) != *self.hosts.initial.borrow()
            || !typed.password.is_empty()
            || typed.password_mode == "remove"
            || !typed.passphrase.is_empty()
            || typed.remove_passphrase
            || !form.get_tag_draft().trim().is_empty()
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
            Next::Quick(target) => self.host_form_open_quick(&target),
        }
    }

    pub(super) fn host_form_close(&self) {
        if let Some(ui) = self.ui.upgrade() {
            let form = ui.global::<HostForm>();
            form.set_open(false);
            // Do not keep a secret in a hidden field.
            form.set_password(SharedString::new());
            form.set_passphrase(SharedString::new());
            form.set_show_password(false);
        }
    }

    /// Show the form: filled in for `host_id`, empty for a new host.
    pub fn host_form_open(&self, host_id: Option<&str>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let host = host_id.and_then(|id| self.inv.host(id));
        let fields = match &host {
            Some(host) => fields_from(host),
            None => Fields {
                port: "22".into(),
                os: "linux".into(),
                ..Fields::default()
            },
        };
        let saved = Saved {
            password: host.as_ref().is_some_and(|host| {
                host.ssh.as_ref().is_some_and(|ssh| ssh.has_password)
                    || host.rdp.as_ref().is_some_and(|rdp| rdp.has_credential)
            }),
            passphrase: host
                .as_ref()
                .and_then(|host| host.ssh.as_ref())
                .is_some_and(|ssh| ssh.has_passphrase),
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
        form.set_protocol(fields.protocol.as_str().into());
        form.set_name(fields.name.as_str().into());
        form.set_address(fields.address.as_str().into());
        form.set_port(fields.port.as_str().into());
        form.set_username(fields.username.as_str().into());
        form.set_os(fields.os.as_str().into());
        form.set_group(fields.group.as_str().into());
        form.set_tag_draft(SharedString::new());
        form.set_tag_error(SharedString::new());
        form.set_snippet(fields.snippet.as_str().into());
        form.set_key_path(fields.key_path.as_str().into());
        form.set_password(SharedString::new());
        form.set_show_password(false);
        form.set_password_mode(if saved.password { "saved" } else { "empty" }.into());
        form.set_password_note(SharedString::new());
        form.set_passphrase(SharedString::new());
        form.set_has_passphrase(saved.passphrase);
        form.set_remove_passphrase(false);
        form.set_error(SharedString::new());
        form.set_saving(false);
        show_problems(&form, &Problems::default());
        *self.hosts.editing.borrow_mut() = host.map(|host| host.id);
        *self.hosts.tags.borrow_mut() = fields.tags.clone();
        *self.hosts.key_id.borrow_mut() = fields.key_id.clone();
        *self.hosts.key_file.borrow_mut() = Default::default();
        self.hosts.saved.set(saved);
        self.hosts.submitted.set(false);
        self.keychain_reload();
        self.host_form_show_method(&form, fields.method);
        self.host_form_show_tags(&form);
        self.host_form_suggest_groups(&form);
        *self.hosts.initial.borrow_mut() = fields;
        self.host_form_follow_key_file(&form);
        form.set_open(true);
        // A new form starts with the cursor in its first field.
        form.set_revision(form.get_revision().wrapping_add(1));
    }

    /// Open a new SSH host form pre-filled from a quick-connect command (`ssh user@host`).
    pub fn host_form_open_quick(&self, target: &crate::inventory::SshTarget) {
        self.host_form_open(None);
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        let port = target.port.to_string();
        let username = target.username.as_deref().unwrap_or("");
        form.set_title("Quick connect".into());
        form.set_address(target.address.as_str().into());
        form.set_port(port.as_str().into());
        form.set_username(username.into());
        // Closing right away without typing credentials should not nag to discard.
        *self.hosts.initial.borrow_mut() = self.host_form_fields(&form);
    }

    /// Offer the existing groups that match what is typed.
    pub(super) fn host_form_suggest_groups(&self, form: &HostForm) {
        let hosts = self.inv.hosts();
        let known = chips::known_groups(hosts.iter().filter_map(|host| host.group_name.as_deref()));
        let offered = chips::group_suggestions(&known, &form.get_group());
        form.set_group_suggestions(chip_rows(&offered, false));
    }

    /// SSH or Remote Desktop, chosen while adding a host. The usual port follows the choice
    /// unless the user typed another one.
    pub(super) fn host_form_set_protocol(&self, protocol: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        if form.get_editing() || form.get_protocol() == protocol {
            return;
        }
        let (usual_before, usual_now) = if protocol == "rdp" {
            ("22", "3389")
        } else {
            ("3389", "22")
        };
        form.set_protocol(protocol.into());
        {
            // Choosing the kind is not typing: an otherwise untouched form stays untouched.
            let mut initial = self.hosts.initial.borrow_mut();
            initial.protocol = if protocol == "rdp" {
                Protocol::Rdp
            } else {
                Protocol::Ssh
            };
            let port = form.get_port();
            if port.trim().is_empty() || port.trim() == usual_before {
                form.set_port(usual_now.into());
                if initial.port == usual_before {
                    initial.port = usual_now.into();
                }
            }
        }
        self.host_form_edited();
    }

    pub(super) fn host_form_edited(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let form = ui.global::<HostForm>();
        // A passphrase typed after ticking "remove" means: replace it.
        if !form.get_passphrase().is_empty() && form.get_remove_passphrase() {
            form.set_remove_passphrase(false);
        }
        self.host_form_suggest_groups(&form);
        self.host_form_follow_key_file(&form);
        let fields = self.host_form_fields(&form);
        // Windows files a Remote Desktop password under the address and the user: once either
        // changes, the saved one is gone and the form says so instead of claiming "saved".
        let moved = self.hosts.editing.borrow().is_some()
            && self.hosts.saved.get().password
            && rdp_password_lost(&self.hosts.initial.borrow(), &fields);
        let mode = form.get_password_mode();
        if moved && mode != "empty" {
            form.set_password_mode("empty".into());
            form.set_password_note(RDP_PASSWORD_MOVED.into());
        } else if !moved && form.get_password_note() == RDP_PASSWORD_MOVED {
            form.set_password_note(SharedString::new());
            if form.get_password().is_empty() {
                form.set_password_mode("saved".into());
            }
        }
        if self.hosts.submitted.get() {
            let problems = self.host_form_check(&form).err().unwrap_or_default();
            show_problems(&form, &problems);
        }
    }
}
