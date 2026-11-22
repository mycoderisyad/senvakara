//! The confirmation dialog: what it asks, what is pending on the answer and how `App` runs it.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{hostform, jobs, keychain, workspace, App, Confirm, KeyValue};

/// What a confirmation dialog asks.
pub struct Question {
    pub title: String,
    pub body: String,
    pub confirm: String,
    pub danger: bool,
}

/// What happens when the user accepts the open confirmation.
pub enum Pending {
    CloseSession(i32),
    /// Host id and the fingerprint the user saw.
    TrustKey(String, String),
    /// A shell command for exactly the phone the dialog named.
    RunShell {
        route_id: String,
        serial: String,
        command: String,
    },
    /// Install a local APK; `label` is what the job list shows.
    InstallApk {
        label: String,
        route_id: String,
        serial: String,
        local_path: String,
    },
    Uninstall {
        route_id: String,
        serial: String,
        package: String,
    },
    /// Route id of a phone that is not connected.
    ForgetDevice(String),
    /// Route id of a Wi-Fi route.
    DisconnectWifi(String),
    /// Start a job again, this time replacing the file that was in the way. Label, job.
    ReplaceFile(String, jobs::JobSpec),
    /// Copies to or from a phone the dialog named: (label, job) each.
    CopyFiles(Vec<(String, jobs::JobSpec)>),
    /// Close the host form although the user typed in it, then go on to something else.
    DiscardHostForm(hostform::Next),
    /// Close the key form although the user typed in it, then go on to something else.
    DiscardKeyForm(keychain::Next),
    /// Id of a key of the Keychain.
    DeleteKey(String),
    /// Host id.
    RemoveHost(String),
    RemoveVpn {
        host_id: String,
        endpoint_id: String,
    },
    /// Database id of a saved workspace.
    DeleteWorkspace(String),
    /// Empty a panel whose session is still going. `workspace` is the pane id.
    CloseWorkspacePanel {
        workspace: i32,
        slot: usize,
    },
    /// Change to a layout that has no room for some panels.
    WorkspaceLayout {
        workspace: i32,
        layout: workspace::Layout,
    },
    /// Start the build-and-deploy the form describes.
    Deploy,
    /// Run the recipe open in the editor.
    RunRecipe,
    /// Database id of a recipe.
    DeleteRecipe(String),
}

impl App {
    /// Show the confirmation dialog; `pending` runs if the user accepts.
    pub fn ask(&self, question: Question, pending: Pending) {
        self.ask_with(question, Vec::new(), "", pending);
    }

    /// Like [`App::ask`], listing exactly what will be affected. An empty confirm label makes
    /// the dialog informational: only Close is offered.
    pub fn ask_with(
        &self,
        question: Question,
        facts: Vec<KeyValue>,
        warning: &str,
        pending: Pending,
    ) {
        let Some(ui) = self.ui.upgrade() else { return };
        let confirm = ui.global::<Confirm>();
        confirm.set_title(question.title.into());
        confirm.set_body(question.body.into());
        confirm.set_confirm_label(question.confirm.into());
        confirm.set_danger(question.danger);
        confirm.set_warning(warning.into());
        confirm.set_facts(ModelRc::new(VecModel::from(facts)));
        *self.pending.borrow_mut() = Some(pending);
        confirm.set_open(true);
    }

    pub(crate) fn answer(self: &Rc<Self>, accepted: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.global::<Confirm>().set_open(false);
        let Some(pending) = self.pending.borrow_mut().take() else {
            return;
        };
        if !accepted {
            if let Pending::TrustKey(host, _) = &pending {
                // The key was not accepted: the terminal that waited for it is not opened.
                self.hosts.take_connect_after_trust(host);
            }
            return;
        }
        match pending {
            Pending::CloseSession(id) => {
                let index = self
                    .sessions
                    .borrow()
                    .iter()
                    .position(|pane| pane.id() == id);
                if let Some(index) = index {
                    self.close(index as i32);
                }
            }
            Pending::TrustKey(host, fingerprint) => self.trust_key(host, fingerprint),
            Pending::RunShell {
                route_id,
                serial,
                command,
            } => self.shell_run(route_id, serial, command),
            Pending::InstallApk {
                label,
                route_id,
                serial,
                local_path,
            } => self.job_start(
                label,
                jobs::JobSpec::InstallApk {
                    route_id,
                    serial,
                    local_path,
                },
            ),
            Pending::Uninstall {
                route_id,
                serial,
                package,
            } => self.uninstall(route_id, serial, package),
            Pending::ForgetDevice(route_id) => self.forget(route_id),
            Pending::DisconnectWifi(route_id) => self.disconnect_wifi(route_id),
            Pending::ReplaceFile(label, spec) => self.job_start(label, spec),
            Pending::CopyFiles(jobs) => {
                for (label, spec) in jobs {
                    self.job_start(label, spec);
                }
            }
            Pending::DiscardHostForm(next) => self.host_form_discard(next),
            Pending::DiscardKeyForm(next) => self.keychain_discard(next),
            Pending::DeleteKey(id) => self.keychain_delete(id),
            Pending::RemoveHost(host_id) => self.host_remove(host_id),
            Pending::RemoveVpn {
                host_id,
                endpoint_id,
            } => self.paths_remove(host_id, endpoint_id),
            Pending::DeleteWorkspace(id) => self.workspace_delete(id),
            Pending::CloseWorkspacePanel { workspace, slot } => {
                self.workspace_clear(workspace, slot)
            }
            Pending::WorkspaceLayout { workspace, layout } => {
                self.workspace_set_layout(workspace, layout)
            }
            Pending::Deploy => self.deploy_start(),
            Pending::RunRecipe => self.recipe_run(),
            Pending::DeleteRecipe(id) => self.recipe_delete(id),
        }
    }
}
