//! The recipe list and editor: opening, editing, showing, saving and deleting a recipe.

use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::inventory::{fact, friendly, friendly_with};
use crate::{App, Pending, Question, RecipeItem, Recipes, RunRow, StepItem};

use super::draft::{
    name_problem, permission_warning, step_problem, Draft, StepDraft, KINDS, MAX_STEPS,
};
use super::history::run_row;
use super::target::device_label;

impl App {
    pub(super) fn recipe_open(self: &Rc<Self>, draft: Draft, stored: bool) {
        *self.auto.stored.borrow_mut() = stored.then(|| draft.clone());
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Recipes>().set_name(draft.name.as_str().into());
        }
        *self.auto.draft.borrow_mut() = Some(draft);
        self.auto.error.borrow_mut().clear();
        self.recipes_publish();
    }

    /// Change the open recipe and redraw.
    pub(super) fn recipe_edit(self: &Rc<Self>, change: impl FnOnce(&mut Draft)) {
        if let Some(draft) = self.auto.draft.borrow_mut().as_mut() {
            change(draft);
        }
        self.recipes_publish();
    }

    pub(super) fn recipe_step(self: &Rc<Self>, step: i32, change: impl FnOnce(&mut StepDraft)) {
        self.recipe_edit(|draft| {
            if let Some(step) = draft.steps.get_mut(step.max(0) as usize) {
                change(step);
            }
        });
    }

    pub(super) fn recipe_step_browse(self: &Rc<Self>, index: usize) {
        let kind = self
            .auto
            .draft
            .borrow()
            .as_ref()
            .and_then(|draft| draft.steps.get(index).map(|step| step.kind.clone()));
        let Some(kind) = kind else { return };
        let wanted = kind.clone();
        let set = move |app: &Rc<App>, path: String| {
            app.recipe_step(index as i32, |step| {
                // The step may have changed kind while the dialog was open.
                if step.kind == wanted {
                    step.text = path;
                }
            });
        };
        match kind.as_str() {
            "install_apk" => {
                let dialog = rfd::FileDialog::new()
                    .set_title("Choose the APK for this step")
                    .add_filter("Android package", &["apk"]);
                self.pick_path(dialog, false, move |app, path| {
                    set(app, path.display().to_string())
                });
            }
            "screenshot" => {
                let dialog =
                    rfd::FileDialog::new().set_title("Choose the folder for the screenshot");
                self.pick_folder(dialog, set);
            }
            "flutter_deploy" => {
                let dialog = rfd::FileDialog::new().set_title("Choose the Flutter project folder");
                self.pick_folder(dialog, set);
            }
            _ => {}
        }
    }

    pub(super) fn recipes_publish(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Recipes>();
        let devices = self.inv.devices();
        let draft = self.auto.draft.borrow();
        let open_id = draft.as_ref().and_then(|draft| draft.id.clone());

        let name_of = |route: &Option<String>| {
            route
                .as_ref()
                .and_then(|route| devices.iter().find(|d| d.record.route_id == *route))
                .map(|device| device.record.display_name.clone())
        };
        let items: Vec<RecipeItem> = self
            .auto
            .recipes
            .borrow()
            .iter()
            .map(|record| {
                let count = record.steps.len();
                let steps = format!("{count} {}", if count == 1 { "step" } else { "steps" });
                RecipeItem {
                    id: record.id.as_str().into(),
                    name: record.name.as_str().into(),
                    detail: match (&record.target_route_id, name_of(&record.target_route_id)) {
                        (_, Some(name)) => format!("{steps}, for {name}"),
                        (Some(_), None) => format!("{steps}, for a forgotten phone"),
                        (None, None) => format!("{steps}, no phone chosen"),
                    }
                    .into(),
                    selected: open_id.as_deref() == Some(record.id.as_str()),
                }
            })
            .collect();
        let current: Vec<RecipeItem> = self.auto.items.iter().collect();
        if current != items {
            self.auto.items.set_vec(items);
        }

        let history = self.auto.history.borrow();
        let rows: Vec<RunRow> = match &*draft {
            Some(draft) => history
                .iter()
                .filter(|run| run.recipe_id.is_some() && run.recipe_id == draft.id)
                .map(|run| run_row(run, "recipe"))
                .collect(),
            None => history
                .iter()
                .filter(|run| run.recipe_id.is_some())
                .map(|run| run_row(run, "recipe"))
                .collect(),
        };
        let current: Vec<RunRow> = view.get_history().iter().collect();
        if current != rows {
            view.set_history(ModelRc::new(VecModel::from(rows)));
        }

        view.set_editing(draft.is_some());
        view.set_error(self.auto.error.borrow().as_str().into());
        let Some(draft) = &*draft else {
            return;
        };

        view.set_saved(draft.id.is_some());
        view.set_title(
            if draft.id.is_some() {
                format!("Edit {}", draft.name.trim())
            } else {
                "New recipe".to_string()
            }
            .into(),
        );
        view.set_name_error(name_problem(&draft.name).into());

        // The target picker: "no phone", then every phone, then the stored one if it is gone.
        let mut routes = vec![String::new()];
        let mut labels: Vec<SharedString> = vec!["Choose a phone".into()];
        for device in &devices {
            routes.push(device.record.route_id.clone());
            labels.push(
                match device.record.state.as_str() {
                    "available" => device_label(device),
                    "unauthorized" => format!("{}, unauthorized", device_label(device)),
                    _ => format!("{}, not connected", device_label(device)),
                }
                .into(),
            );
        }
        if !routes.contains(&draft.route_id) {
            routes.push(draft.route_id.clone());
            labels.push("A phone that was forgotten".into());
        }
        let chosen = routes
            .iter()
            .position(|route| *route == draft.route_id)
            .unwrap_or(0);
        let current: Vec<SharedString> = view.get_targets().iter().collect();
        if current != labels {
            view.set_targets(ModelRc::new(VecModel::from(labels)));
        }
        view.set_target_index(chosen as i32);
        *self.auto.recipe_routes.borrow_mut() = routes;

        view.set_allow_install(draft.install);
        view.set_allow_shell(draft.shell);

        let count = draft.steps.len();
        let steps: Vec<StepItem> = draft
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| StepItem {
                kind_index: KINDS
                    .iter()
                    .position(|(kind, _)| *kind == step.kind)
                    .map(|position| position as i32)
                    .unwrap_or(-1),
                kind: step.kind.as_str().into(),
                text: step.text.as_str().into(),
                seconds: step.seconds.as_str().into(),
                mode: step.mode.as_str().into(),
                launch: step.launch,
                error: step_problem(step).into(),
                first: index == 0,
                last: index + 1 == count,
            })
            .collect();
        if self.auto.steps.row_count() != steps.len() {
            self.auto.steps.set_vec(steps);
        } else {
            for (index, step) in steps.into_iter().enumerate() {
                if self.auto.steps.row_data(index).as_ref() != Some(&step) {
                    self.auto.steps.set_row_data(index, step);
                }
            }
        }

        view.set_add_reason(
            if count >= MAX_STEPS {
                "A recipe can have at most 20 steps."
            } else {
                ""
            }
            .into(),
        );
        view.set_remove_reason(
            if count <= 1 {
                "A recipe needs at least one step, so the only step cannot be removed."
            } else {
                ""
            }
            .into(),
        );

        let dirty = self.auto.stored.borrow().as_ref() != Some(draft);
        let problem = draft.save_problem();
        view.set_save_reason(
            if !problem.is_empty() {
                problem
            } else if !dirty {
                "There are no changes to save."
            } else {
                ""
            }
            .into(),
        );
        let warning = permission_warning(&draft.steps, draft.install, draft.shell);
        let target = devices
            .iter()
            .find(|device| device.record.route_id == draft.route_id);
        let target_warning = match target {
            _ if draft.route_id.is_empty() => String::new(),
            None => "The target phone was forgotten. Choose another phone to run this recipe."
                .to_string(),
            Some(device) => match device.record.state.as_str() {
                "available" => String::new(),
                "unauthorized" => format!(
                    "{} has not allowed USB debugging yet. Unlock it and accept the prompt to run this recipe.",
                    device.record.display_name
                ),
                _ => format!(
                    "{} is not connected. Connect it to run this recipe.",
                    device.record.display_name
                ),
            },
        };
        view.set_run_reason(
            if !problem.is_empty() {
                problem
            } else if draft.route_id.is_empty() {
                "Choose a target phone to run the recipe."
            } else if !target_warning.is_empty() {
                "The target phone is not ready."
            } else if !warning.is_empty() {
                "A step needs a permission that is not ticked."
            } else {
                ""
            }
            .into(),
        );
        view.set_permission_warning(warning.into());
        view.set_target_warning(target_warning.into());
    }

    /// Store the open recipe. Returns its id when it is stored (also when nothing changed).
    pub(super) fn recipe_save(self: &Rc<Self>) -> Option<String> {
        let draft = self.auto.draft.borrow().clone()?;
        if !draft.save_problem().is_empty() {
            return None;
        }
        if self.auto.stored.borrow().as_ref() == Some(&draft) {
            return draft.id;
        }
        let saved = match self.backend.recipe_save(draft.to_save()) {
            Ok(record) => {
                let stored = Draft {
                    id: Some(record.id.clone()),
                    ..draft
                };
                // Keep what is in the editor (it is what was sent), now with its id.
                *self.auto.draft.borrow_mut() = Some(stored.clone());
                *self.auto.stored.borrow_mut() = Some(stored);
                self.auto.error.borrow_mut().clear();
                Some(record.id)
            }
            Err(error) => {
                *self.auto.error.borrow_mut() =
                    friendly_with(&error, "This recipe was deleted in the meantime.");
                None
            }
        };
        self.automations_reload();
        saved
    }

    pub(super) fn recipe_remove_ask(self: &Rc<Self>) {
        let draft = self.auto.draft.borrow().clone();
        let Some((draft, id)) = draft.and_then(|draft| draft.id.clone().map(|id| (draft, id)))
        else {
            return;
        };
        self.ask_with(
            Question {
                title: format!("Delete {}?", draft.name.trim()),
                body: "The recipe is removed. Its past runs stay in the history.".into(),
                confirm: "Delete recipe".into(),
                danger: true,
            },
            vec![fact("Recipe", draft.name.trim(), false)],
            "",
            Pending::DeleteRecipe(id),
        );
    }

    /// The user confirmed deleting a recipe.
    pub fn recipe_delete(self: &Rc<Self>, id: String) {
        match self.backend.recipe_delete(id.clone()) {
            Ok(()) => {
                let open = self.auto.draft.borrow().as_ref().and_then(|d| d.id.clone());
                if open.as_deref() == Some(id.as_str()) {
                    *self.auto.draft.borrow_mut() = None;
                    *self.auto.stored.borrow_mut() = None;
                }
                self.auto.error.borrow_mut().clear();
            }
            Err(error) => *self.auto.error.borrow_mut() = friendly(&error),
        }
        self.automations_reload();
    }
}
