//! The inspector: the selected node's machine, settings, connections and script, or the
//! workflow's name, permissions and problems. Fields are refilled only when the selection or
//! a non-text value changes, so typing never loses the cursor.

use std::rc::Rc;

use senvakara_app::workflow::{Binding, EdgeKind, Executor, Node, ParamKind, SCRIPT_FILES};
use senvakara_app::{Catalog, Workflow};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::edit;
use super::view::{problems_by_node, title_of};
use crate::{App, WfField, WfLink, WfProblem};

/// Machines an anchor can stand for, in picker order after the "choose" entry.
fn machine_choices(app: &App, module: &str) -> Vec<(Binding, String)> {
    let machines = app.wf.machines.borrow();
    let mut out: Vec<(Binding, String)> = match module {
        "host.ssh" => machines
            .hosts
            .iter()
            .map(|(id, m)| {
                (
                    Binding::Host {
                        host_id: id.clone(),
                    },
                    m.name.clone(),
                )
            })
            .collect(),
        "android.device" => machines
            .devices
            .iter()
            .map(|(id, m)| {
                (
                    Binding::Device {
                        route_id: id.clone(),
                    },
                    m.name.clone(),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out
}

/// Script files the inspector offers for a node.
fn script_files(catalog: &Catalog, node: &Node) -> Vec<&'static str> {
    let Some(m) = catalog.get(&node.module) else {
        return Vec::new();
    };
    if m.manifest.executor != Executor::SshScript {
        return Vec::new();
    }
    if m.manifest.user_scripts {
        return vec!["apply.sh", "check.sh"];
    }
    SCRIPT_FILES
        .into_iter()
        .filter(|f| m.files.contains_key(*f))
        .collect()
}

fn field_rows(catalog: &Catalog, node: &Node) -> Vec<WfField> {
    let Some(m) = catalog.get(&node.module) else {
        return Vec::new();
    };
    m.manifest
        .params
        .iter()
        .map(|p| {
            let value = node.params.get(&p.name).or(p.default.as_ref());
            let text = match value {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            let kind = match p.kind {
                ParamKind::Bool => "bool",
                ParamKind::Choice => "choice",
                ParamKind::Multiline => "multiline",
                ParamKind::Secret => "secret",
                _ => "text",
            };
            let options: Vec<SharedString> = p.options.iter().map(|o| o.as_str().into()).collect();
            // A secret shows only whether one is stored; its reference stays in Rust.
            let secret = p.kind == ParamKind::Secret;
            let stored = secret && text.starts_with("secret-");
            let text = if secret { String::new() } else { text };
            WfField {
                name: p.name.clone().into(),
                label: p.label.clone().into(),
                kind: kind.into(),
                checked: stored || value.and_then(|v| v.as_bool()).unwrap_or(false),
                index: p
                    .options
                    .iter()
                    .position(|o| *o == text)
                    .map_or(0, |i| i as i32),
                options: ModelRc::new(VecModel::from(options)),
                value: text.into(),
                help: p.help.clone().unwrap_or_default().into(),
                browse: match (p.kind, &p.suffix) {
                    (ParamKind::Path, Some(_)) => "file",
                    (ParamKind::Path, None) => "folder",
                    _ => "",
                }
                .into(),
            }
        })
        .collect()
}

/// The selected node's connections, worded from its side.
fn link_rows(wf: &Workflow, catalog: &Catalog, id: &str) -> Vec<WfLink> {
    let name = |nid: &str| {
        wf.node(nid)
            .map(|n| title_of(n, catalog))
            .unwrap_or_default()
    };
    wf.edges
        .iter()
        .filter(|e| e.from == id || e.to == id)
        .map(|e| {
            let text = match (e.kind, e.from == id) {
                (EdgeKind::Target, true) => format!("Runs on {}", name(&e.to)),
                (EdgeKind::Target, false) => format!("{} runs here", name(&e.from)),
                (EdgeKind::After, true) => format!("{} runs after this", name(&e.to)),
                (EdgeKind::After, false) => format!("Runs after {}", name(&e.from)),
                (EdgeKind::DependsOn, true) => format!("{} uses this service", name(&e.to)),
                (EdgeKind::DependsOn, false) => format!("Uses the service of {}", name(&e.from)),
            };
            WfLink {
                from: e.from.clone().into(),
                to: e.to.clone().into(),
                text: text.into(),
                orders: e.kind.orders(),
                uses_service: e.kind == EdgeKind::DependsOn,
            }
        })
        .collect()
}

impl App {
    /// Fills the inspector. `fields` false keeps text fields as typed and only updates what
    /// they do not hold (problems, connections, the card at the top).
    pub(super) fn workflows_fill_inspector(self: &Rc<Self>, fields: bool) {
        let catalog = senvakara_app::catalog();
        let selected = self.wf.selected.borrow().clone();
        let open = self.wf.open.borrow();
        let Some(o) = open.as_ref() else { return };
        let perms = o.graph.permissions;
        let node = o.graph.node(&selected).cloned();
        let row = o
            .graph
            .nodes
            .iter()
            .position(|n| n.id == selected)
            .and_then(|i| self.wf.nodes.row_data(i));
        let by_node = problems_by_node(&self.wf.problems.borrow());
        let sel_problems: Vec<WfProblem> = by_node
            .get(&selected)
            .into_iter()
            .flatten()
            .map(|t| WfProblem {
                node: selected.clone().into(),
                text: t.clone().into(),
            })
            .collect();
        let links = node
            .as_ref()
            .map(|_| link_rows(&o.graph, catalog, &selected))
            .unwrap_or_default();
        drop(open);
        self.wf.sel_problems.set_vec(sel_problems);
        self.wf.links.set_vec(links);

        let Some(node) = node else {
            self.with_view(|v| {
                v.set_selected("".into());
                v.set_allow_root(perms.root);
                v.set_allow_shell(perms.shell);
                v.set_allow_install(perms.install);
                v.set_allow_network(perms.network);
            });
            return;
        };
        let module = catalog.get(&node.module);
        self.workflows_fill_security(&node);
        self.workflows_fill_service(&node);
        self.with_view(|v| {
            if let Some(row) = row {
                v.set_sel(row);
            }
            v.set_sel_summary(
                module
                    .map(|m| m.manifest.summary.clone())
                    .unwrap_or_default()
                    .into(),
            );
        });
        if !fields {
            return;
        }
        let picks = matches!(node.module.as_str(), "host.ssh" | "android.device");
        let choices = machine_choices(self, &node.module);
        let (label, none) = if node.module == "android.device" {
            ("Device", "Choose a device")
        } else {
            ("Host", "Choose a host")
        };
        let mut names: Vec<SharedString> = vec![none.into()];
        names.extend(choices.iter().map(|(_, n)| SharedString::from(n.as_str())));
        let index = node
            .binding
            .as_ref()
            .and_then(|b| choices.iter().position(|(c, _)| c == b))
            .map_or(0, |i| i as i32 + 1);
        self.wf.fields.set_vec(field_rows(catalog, &node));
        let files = script_files(catalog, &node);
        let shown = self.wf.script.get().min(files.len().saturating_sub(1));
        let (text, custom) = match files.get(shown) {
            Some(f) => {
                let edited = node.scripts.get(*f);
                let bundled = module.and_then(|m| m.files.get(*f));
                let user = module.is_some_and(|m| m.manifest.user_scripts);
                (
                    edited.or(bundled).cloned().unwrap_or_default(),
                    edited.is_some() && !user,
                )
            }
            None => (String::new(), false),
        };
        let script_names: Vec<SharedString> =
            files.iter().map(|f| SharedString::from(*f)).collect();
        let (connect_ids, connect_names) = self.connect_choices(&node.id);
        *self.wf.connect_ids.borrow_mut() = connect_ids;
        self.with_view(|v| {
            v.set_sel_title(node.title.clone().into());
            v.set_sel_picks_machine(picks);
            v.set_machine_label(label.into());
            v.set_machines(ModelRc::new(VecModel::from(names)));
            v.set_machine_index(index);
            v.set_scripts(ModelRc::new(VecModel::from(script_names)));
            v.set_script_index(shown as i32);
            v.set_script_text(text.into());
            v.set_script_custom(custom);
            v.set_connect_options(ModelRc::new(VecModel::from(connect_names)));
        });
    }

    /// Other nodes, by title, for the "Connect to" picker: (ids, names after a first
    /// "Choose a node" entry).
    fn connect_choices(&self, id: &str) -> (Vec<String>, Vec<SharedString>) {
        let catalog = senvakara_app::catalog();
        let open = self.wf.open.borrow();
        let Some(o) = open.as_ref() else {
            return (Vec::new(), Vec::new());
        };
        let mut others: Vec<(String, String)> = o
            .graph
            .nodes
            .iter()
            .filter(|n| n.id != id)
            .map(|n| {
                let kind = catalog
                    .get(&n.module)
                    .map(|m| m.manifest.category.label())
                    .unwrap_or("Unknown");
                (n.id.clone(), format!("{} ({kind})", title_of(n, catalog)))
            })
            .collect();
        others.sort_by_key(|(_, name)| name.to_lowercase());
        let mut names = vec![SharedString::from("Choose a node")];
        names.extend(others.iter().map(|(_, n)| SharedString::from(n.as_str())));
        (others.into_iter().map(|(id, _)| id).collect(), names)
    }

    /// Connects the selected node to the picked one, as a drag on the canvas would.
    pub(super) fn workflows_connect_to(self: &Rc<Self>, index: i32) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let Some(other) = self
            .wf
            .connect_ids
            .borrow()
            .get((index - 1).max(0) as usize)
            .cloned()
        else {
            return;
        };
        let mut result = Ok(());
        self.workflows_edit(|wf| {
            result = edit::connect(wf, senvakara_app::catalog(), &id, &other).map(|_| ());
        });
        match result {
            Ok(()) => {
                self.workflows_error("");
                self.workflows_refresh(true);
            }
            Err(text) => {
                self.workflows_error(&text);
                // Back to "Choose a node".
                self.workflows_fill_inspector(true);
            }
        }
    }

    fn selected_node(&self) -> Option<String> {
        let id = self.wf.selected.borrow().clone();
        (!id.is_empty()).then_some(id)
    }

    pub(super) fn workflows_set_title(self: &Rc<Self>, text: &str) {
        let Some(id) = self.selected_node() else {
            return;
        };
        self.workflows_edit(|wf| edit::set_title(wf, &id, text));
        self.workflows_refresh(false);
    }

    pub(super) fn workflows_pick_machine(self: &Rc<Self>, index: i32) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let module = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(&id).map(|n| n.module.clone()))
            .unwrap_or_default();
        let choices = machine_choices(self, &module);
        let binding = (index > 0)
            .then(|| choices.get(index as usize - 1).map(|(b, _)| b.clone()))
            .flatten();
        self.workflows_edit(|wf| edit::set_binding(wf, &id, binding));
        self.workflows_refresh(true);
    }

    pub(super) fn workflows_field_text(self: &Rc<Self>, name: &str, text: &str) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let catalog = senvakara_app::catalog();
        let numeric = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(&id).map(|n| n.module.clone()))
            .and_then(|m| catalog.get(&m).and_then(|m| m.param(name)).map(|p| p.kind))
            .is_some_and(|k| matches!(k, ParamKind::Int | ParamKind::Port));
        // A number stays a number; anything else is kept as typed and checked by validation.
        let value = match text.trim().parse::<i64>() {
            Ok(n) if numeric => serde_json::json!(n),
            _ => serde_json::json!(text),
        };
        self.workflows_edit(|wf| edit::set_param(wf, &id, name, value));
        self.workflows_refresh(false);
    }

    pub(super) fn workflows_field_checked(self: &Rc<Self>, name: &str, on: bool) {
        let Some(id) = self.selected_node() else {
            return;
        };
        self.workflows_edit(|wf| edit::set_param(wf, &id, name, serde_json::json!(on)));
        self.workflows_refresh(true);
    }

    pub(super) fn workflows_field_choice(self: &Rc<Self>, name: &str, index: i32) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let option = self
            .wf
            .fields
            .iter()
            .find(|f| f.name == name)
            .and_then(|f| {
                f.options
                    .row_data(index.max(0) as usize)
                    .map(|s| s.to_string())
            });
        if let Some(option) = option {
            self.workflows_edit(|wf| edit::set_param(wf, &id, name, serde_json::json!(option)));
            self.workflows_refresh(true);
        }
    }

    /// Browse for the path of a setting on this PC: a file when the setting names its ending
    /// (`.apk`), else a folder.
    pub(super) fn workflows_field_browse(self: &Rc<Self>, name: &str) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let spec = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(&id).map(|n| n.module.clone()))
            .and_then(|m| {
                senvakara_app::catalog()
                    .get(&m)
                    .and_then(|m| m.param(name))
                    .cloned()
            });
        let Some(spec) = spec else { return };
        let name = name.to_string();
        let store = move |app: &Rc<App>, path: String| {
            app.workflows_edit(|wf| edit::set_param(wf, &id, &name, serde_json::json!(path)));
            app.workflows_refresh(true);
        };
        match spec.suffix {
            Some(suffix) => {
                let ext = suffix.trim_start_matches('.').to_string();
                let dialog = rfd::FileDialog::new()
                    .set_title(&spec.label)
                    .add_filter(ext.to_uppercase(), &[ext.as_str()]);
                self.pick_path(dialog, false, move |app, path| {
                    store(app, path.display().to_string())
                });
            }
            None => {
                let dialog = rfd::FileDialog::new().set_title(&spec.label);
                self.pick_folder(dialog, store);
            }
        }
    }

    pub(super) fn workflows_pick_script(self: &Rc<Self>, index: i32) {
        self.wf.script.set(index.max(0) as usize);
        self.workflows_fill_inspector(true);
    }

    fn shown_script(&self, id: &str) -> Option<&'static str> {
        let open = self.wf.open.borrow();
        let node = open.as_ref()?.graph.node(id)?;
        let files = script_files(senvakara_app::catalog(), node);
        files.get(self.wf.script.get()).copied()
    }

    pub(super) fn workflows_script_edited(self: &Rc<Self>, text: &str) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let Some(file) = self.shown_script(&id) else {
            return;
        };
        self.workflows_edit(|wf| edit::set_script(wf, senvakara_app::catalog(), &id, file, text));
        let custom = {
            let open = self.wf.open.borrow();
            open.as_ref()
                .and_then(|o| o.graph.node(&id))
                .is_some_and(|n| {
                    n.scripts.contains_key(file)
                        && senvakara_app::catalog()
                            .get(&n.module)
                            .is_some_and(|m| !m.manifest.user_scripts)
                })
        };
        self.with_view(|v| v.set_script_custom(custom));
        self.workflows_refresh(false);
    }

    pub(super) fn workflows_script_reset(self: &Rc<Self>) {
        let Some(id) = self.selected_node() else {
            return;
        };
        let Some(file) = self.shown_script(&id) else {
            return;
        };
        self.workflows_edit(|wf| {
            if let Some(n) = wf.nodes.iter_mut().find(|n| n.id == id) {
                n.scripts.remove(file);
            }
        });
        self.workflows_refresh(true);
    }

    pub(super) fn workflows_link_remove(self: &Rc<Self>, from: &str, to: &str) {
        self.workflows_edit(|wf| edit::disconnect(wf, from, to));
        self.workflows_refresh(true);
    }

    pub(super) fn workflows_link_service(self: &Rc<Self>, from: &str, to: &str, on: bool) {
        self.workflows_edit(|wf| edit::set_order_kind(wf, from, to, on));
        self.workflows_refresh(true);
    }
}
