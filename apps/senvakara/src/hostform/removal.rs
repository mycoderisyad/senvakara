//! Removing a host, after asking.

use std::rc::Rc;

use senvakara_app::HostView;
use slint::ComponentHandle;

use crate::inventory::{fact, friendly_with};
use crate::{App, Inventory, Pending, Question};

impl App {
    pub fn host_remove_ask(self: &Rc<Self>, host: &HostView) {
        let mut facts = vec![fact("Host", host.display_name.as_str(), false)];
        if let Some(ssh) = &host.ssh {
            facts.push(fact(
                "Address",
                format!("{}@{}:{}", ssh.username, ssh.address, ssh.port),
                true,
            ));
        }
        if let Some(rdp) = &host.rdp {
            facts.push(fact(
                "Address",
                format!("{}:{}", rdp.address, rdp.port),
                true,
            ));
        }
        self.ask_with(
            Question {
                title: format!("Remove {}?", host.display_name),
                body: "Its settings, its trusted key, the saved password or passphrase and its VPN addresses are removed from Senvakara and from Windows Credential Manager, and open sessions on it end. A key in the Keychain stays. Nothing changes on the host itself.".into(),
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
}
