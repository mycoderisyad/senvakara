//! Adding, editing and removing a remote host, and the addresses it can be reached at. The
//! form lives in the inspector; nothing is saved until Save, and a form the user has typed in
//! is never dropped without asking.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_app::{CoreError, HostUpsert, HostView};
use slint::{ComponentHandle, SharedString};

use crate::inventory::{fact, friendly_with};
use crate::{App, HostForm, Inventory, NetPaths, Pending, Question};

/// A host may have this many VPN addresses (the storage layer's limit).
const MAX_VPN: usize = 4;

/// What the form holds, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub name: String,
    pub address: String,
    pub port: String,
    pub username: String,
    /// "linux" or "unknown".
    pub os: String,
    pub key_path: String,
}

/// What is wrong with each field, in words. An empty string means the field is fine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub name: &'static str,
    pub address: &'static str,
    pub port: &'static str,
    pub username: &'static str,
    pub key: &'static str,
}

impl Problems {
    pub fn any(&self) -> bool {
        [self.name, self.address, self.port, self.username, self.key]
            .iter()
            .any(|problem| !problem.is_empty())
    }
}

/// Why `text` is not a host name or IP address, or "" when it is.
pub fn address_problem(text: &str) -> &'static str {
    let text = text.trim();
    if text.is_empty() {
        "Enter the host name or IP address."
    } else if text.chars().any(char::is_whitespace) {
        "An address cannot contain spaces."
    } else if text.contains('@') {
        "Enter only the address here. The user name has its own field."
    } else if text.contains('/') {
        "Enter a host name or IP address, not a link."
    } else if text.len() > 253 {
        "This address is too long."
    } else {
        ""
    }
}

/// The port as a number, or why it is not one.
pub fn parse_port(text: &str) -> Result<u16, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter the SSH port. The usual one is 22.");
    }
    match text.parse::<u32>() {
        Ok(port @ 1..=65535) => Ok(port as u16),
        _ => Err("The port must be a number between 1 and 65535."),
    }
}

/// Check the form. Ok carries what the backend stores.
pub fn check(fields: &Fields) -> Result<HostUpsert, Problems> {
    let mut problems = Problems::default();
    let name = fields.name.trim();
    if name.is_empty() {
        problems.name = "Give the host a name.";
    }
    problems.address = address_problem(&fields.address);
    let port = parse_port(&fields.port);
    if let Err(problem) = port {
        problems.port = problem;
    }
    let username = fields.username.trim();
    if username.is_empty() {
        problems.username = "Enter the user name to sign in with.";
    } else if username.chars().any(char::is_whitespace) {
        problems.username = "A user name cannot contain spaces.";
    }
    let key = fields.key_path.trim();
    if key.is_empty() {
        problems.key = "Choose the private key file to sign in with.";
    }
    match port {
        Ok(port) if !problems.any() => Ok(HostUpsert {
            display_name: name.to_string(),
            os_family: if fields.os == "unknown" {
                "unknown"
            } else {
                "linux"
            }
            .to_string(),
            address: fields.address.trim().to_string(),
            port,
            username: username.to_string(),
            private_key_path: key.to_string(),
        }),
        _ => Err(problems),
    }
}

/// Why a VPN address cannot be added, or "" when it can. `known` are the addresses the host
/// already has (its main one included); `vpn_count` how many of them are VPN addresses.
pub fn vpn_problem(address: &str, port: &str, known: &[String], vpn_count: usize) -> &'static str {
    let address = address.trim();
    if vpn_count >= MAX_VPN {
        return "A host can have at most 4 VPN addresses. Remove one first.";
    }
    if address.is_empty() {
        return "Type the VPN address of this host first.";
    }
    let problem = address_problem(address);
    if !problem.is_empty() {
        return problem;
    }
    if known.iter().any(|have| have.eq_ignore_ascii_case(address)) {
        return "This host already has that address.";
    }
    match parse_port(port) {
        Ok(_) => "",
        Err(problem) => problem,
    }
}

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

    // ------------------------------------------------------------- the form

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

    fn host_form_close(&self) {
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

    fn host_form_edited(&self) {
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

    fn host_form_browse(self: &Rc<Self>) {
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

    fn host_form_save(self: &Rc<Self>) {
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

    // -------------------------------------------------------------- removal

    pub fn host_remove_ask(self: &Rc<Self>, host: &HostView) {
        let mut facts = vec![fact("Host", host.display_name.as_str(), false)];
        if let Some(ssh) = &host.ssh {
            facts.push(fact(
                "Address",
                format!("{}@{}:{}", ssh.username, ssh.address, ssh.port),
                true,
            ));
        }
        self.ask_with(
            Question {
                title: format!("Remove {}?", host.display_name),
                body: "Its settings, its trusted key, the saved passphrase and its VPN addresses are removed from Senvakara, and open sessions on it end. Nothing changes on the host itself.".into(),
                confirm: "Remove host".into(),
                danger: true,
            },
            facts,
            "",
            Pending::RemoveHost(host.id.clone()),
        );
    }

    pub fn host_remove(self: &Rc<Self>, host_id: String) {
        let backend = self.backend.clone();
        self.run(
            async move { backend.host_delete(host_id).await },
            |app, result| {
                app.host_form_close();
                app.reload();
                if let (Err(error), Some(ui)) = (result, app.ui.upgrade()) {
                    ui.global::<Inventory>()
                        .set_error(friendly_with(&error, "The host could not be removed.").into());
                }
            },
        );
    }

    // -------------------------------------------------------- network paths

    /// Called whenever the selection is published: another host starts with empty fields.
    pub fn paths_sync(&self, host_id: &str) {
        if *self.hosts.paths_host.borrow() == host_id {
            return;
        }
        *self.hosts.paths_host.borrow_mut() = host_id.to_string();
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let port = self
            .inv
            .host(host_id)
            .and_then(|host| host.ssh.map(|ssh| ssh.port))
            .unwrap_or(22);
        paths.set_address(SharedString::new());
        paths.set_port(port.to_string().into());
        paths.set_error(SharedString::new());
        self.paths_validate();
    }

    /// (host, why the typed address cannot be added)
    fn paths_check(&self, address: &str, port: &str) -> Option<(HostView, &'static str)> {
        let host = self.inv.host(&self.hosts.paths_host.borrow())?;
        let mut known: Vec<String> = host.vpn.iter().map(|vpn| vpn.address.clone()).collect();
        if let Some(ssh) = &host.ssh {
            known.push(ssh.address.clone());
        }
        let problem = vpn_problem(address, port, &known, host.vpn.len());
        Some((host, problem))
    }

    fn paths_validate(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let reason = self
            .paths_check(&paths.get_address(), &paths.get_port())
            .map(|(_, problem)| problem)
            .unwrap_or("Select a host first.");
        paths.set_add_reason(reason.into());
    }

    /// Add the typed address, or `hint`: an address the host itself reported, on its SSH port.
    fn paths_add(self: &Rc<Self>, hint: Option<String>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let paths = ui.global::<NetPaths>();
        let (address, port) = match hint {
            Some(address) => {
                let port = self
                    .inv
                    .host(&self.hosts.paths_host.borrow())
                    .and_then(|host| host.ssh.map(|ssh| ssh.port))
                    .unwrap_or(22);
                (address, port.to_string())
            }
            None => (
                paths.get_address().to_string(),
                paths.get_port().to_string(),
            ),
        };
        let Some((host, problem)) = self.paths_check(&address, &port) else {
            return;
        };
        if !problem.is_empty() {
            paths.set_error(problem.into());
            return;
        }
        let Ok(port) = parse_port(&port) else { return };
        match self
            .backend
            .host_add_vpn(host.id, address.trim().to_string(), port)
        {
            Ok(_) => {
                paths.set_address(SharedString::new());
                paths.set_error(SharedString::new());
            }
            Err(error) => {
                paths.set_error(friendly_with(&error, "This host already has that address.").into())
            }
        }
        self.reload();
        self.paths_validate();
    }

    fn paths_remove_ask(self: &Rc<Self>, endpoint_id: &str) {
        let Some(host) = self.inv.host(&self.hosts.paths_host.borrow()) else {
            return;
        };
        let Some(endpoint) = host.vpn.iter().find(|vpn| vpn.endpoint_id == endpoint_id) else {
            return;
        };
        self.ask_with(
            Question {
                title: "Remove this VPN address?".into(),
                body: "Senvakara stops trying this address when the host does not answer on its main one. The VPN itself is not changed.".into(),
                confirm: "Remove address".into(),
                danger: true,
            },
            vec![
                fact("Host", host.display_name.as_str(), false),
                fact(
                    "VPN address",
                    format!("{}:{}", endpoint.address, endpoint.port),
                    true,
                ),
            ],
            "",
            Pending::RemoveVpn {
                host_id: host.id.clone(),
                endpoint_id: endpoint_id.to_string(),
            },
        );
    }

    pub fn paths_remove(self: &Rc<Self>, host_id: String, endpoint_id: String) {
        let result = self.backend.host_remove_vpn(host_id, endpoint_id);
        if let (Err(error), Some(ui)) = (result, self.ui.upgrade()) {
            ui.global::<NetPaths>()
                .set_error(friendly_with(&error, "The address could not be removed.").into());
        }
        self.reload();
        self.paths_validate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled() -> Fields {
        Fields {
            name: " Fedora laptop ".into(),
            address: "192.0.2.10".into(),
            port: "22".into(),
            username: "alice".into(),
            os: "linux".into(),
            key_path: "C:\\Users\\alice\\.ssh\\id_ed25519".into(),
        }
    }

    #[test]
    fn a_complete_form_is_accepted_and_trimmed() {
        let input = check(&filled()).expect("valid");
        assert_eq!(input.display_name, "Fedora laptop");
        assert_eq!((input.address.as_str(), input.port), ("192.0.2.10", 22));
        assert_eq!(input.os_family, "linux");
        let other = check(&Fields {
            os: "unknown".into(),
            port: " 2222 ".into(),
            ..filled()
        })
        .expect("valid");
        assert_eq!((other.os_family.as_str(), other.port), ("unknown", 2222));
    }

    #[test]
    fn an_empty_form_names_every_field() {
        let problems = check(&Fields::default()).expect_err("invalid");
        assert!(problems.name.starts_with("Give the host"));
        assert!(problems.address.starts_with("Enter the host name"));
        assert!(problems.port.starts_with("Enter the SSH port"));
        assert!(problems.username.starts_with("Enter the user name"));
        assert!(problems.key.starts_with("Choose the private key"));
    }

    #[test]
    fn each_field_is_checked_on_its_own() {
        let only = |fields: Fields| check(&fields).expect_err("invalid");
        let port = only(Fields {
            port: "0".into(),
            ..filled()
        });
        assert_eq!(port.port, "The port must be a number between 1 and 65535.");
        assert_eq!(
            port,
            Problems {
                port: port.port,
                ..Problems::default()
            }
        );
        for bad in ["65536", "ssh", "-1", "22.5"] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_port("65535"), Ok(65535));
        assert_eq!(parse_port("1"), Ok(1));

        assert_eq!(
            only(Fields {
                address: "my host".into(),
                ..filled()
            })
            .address,
            "An address cannot contain spaces."
        );
        assert!(only(Fields {
            address: "alice@host".into(),
            ..filled()
        })
        .address
        .starts_with("Enter only the address"));
        assert!(only(Fields {
            address: "ssh://host".into(),
            ..filled()
        })
        .address
        .contains("not a link"));
        assert_eq!(
            only(Fields {
                username: "al ice".into(),
                ..filled()
            })
            .username,
            "A user name cannot contain spaces."
        );
    }

    #[test]
    fn vpn_addresses() {
        let known = vec!["192.0.2.10".to_string(), "10.8.0.3".to_string()];
        assert_eq!(vpn_problem("100.64.0.7", "22", &known, 1), "");
        assert_eq!(vpn_problem("my-host.tailnet", " 2222 ", &known, 1), "");
        assert!(vpn_problem("", "22", &known, 1).starts_with("Type the VPN address"));
        assert_eq!(
            vpn_problem("10.8.0.3", "22", &known, 1),
            "This host already has that address."
        );
        assert_eq!(
            vpn_problem("10.8.0.9", "70000", &known, 1),
            "The port must be a number between 1 and 65535."
        );
        assert!(vpn_problem("10.8.0.9", "22", &known, 4).starts_with("A host can have at most"));
        assert_eq!(
            vpn_problem("10.8 .0.9", "22", &known, 1),
            "An address cannot contain spaces."
        );
    }
}
