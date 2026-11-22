//! The Automations section as a whole: wiring its callbacks and reading what it shows.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::inventory::friendly;
use crate::{App, Deploy, Recipes};

use super::draft::{Draft, StepDraft, KINDS, MAX_STEPS};

impl App {
    pub fn init_automations(self: &Rc<Self>, ui: &crate::MainWindow) {
        let deploy = ui.global::<Deploy>();
        deploy.on_edited({
            let app = self.clone();
            move || app.deploy_publish()
        });
        deploy.on_browse({
            let app = self.clone();
            move || {
                let dialog = rfd::FileDialog::new().set_title("Choose the Flutter project folder");
                app.pick_folder(dialog, |app, path| {
                    if let Some(ui) = app.ui.upgrade() {
                        ui.global::<Deploy>().set_project(path.into());
                    }
                    app.deploy_publish();
                });
            }
        });
        deploy.on_set_mode({
            let app = self.clone();
            move |mode| {
                *app.auto.mode.borrow_mut() = mode.to_string();
                app.deploy_publish();
            }
        });
        deploy.on_pick_target({
            let app = self.clone();
            move |index| {
                let route = app
                    .auto
                    .deploy_routes
                    .borrow()
                    .get(index.max(0) as usize)
                    .cloned();
                if let Some(route) = route {
                    *app.auto.deploy_route.borrow_mut() = route;
                }
                app.deploy_publish();
            }
        });
        deploy.on_set_launch({
            let app = self.clone();
            move |on| {
                app.auto.launch.set(on);
                app.deploy_publish();
            }
        });
        deploy.on_set_logcat({
            let app = self.clone();
            move |on| {
                app.auto.logcat.set(on);
                app.deploy_publish();
            }
        });
        deploy.on_start({
            let app = self.clone();
            move || app.deploy_ask()
        });

        let recipes = ui.global::<Recipes>();
        recipes.set_steps(self.auto.steps.clone().into());
        recipes.set_items(self.auto.items.clone().into());
        recipes.set_kinds(ModelRc::new(VecModel::from(
            KINDS
                .iter()
                .map(|(_, label)| SharedString::from(*label))
                .collect::<Vec<_>>(),
        )));
        recipes.on_select({
            let app = self.clone();
            move |id| {
                let record = app
                    .auto
                    .recipes
                    .borrow()
                    .iter()
                    .find(|record| record.id == id.as_str())
                    .cloned();
                if let Some(record) = record {
                    app.recipe_open(Draft::from_record(&record), true);
                }
            }
        });
        recipes.on_create({
            let app = self.clone();
            move || {
                let route = app
                    .inv
                    .available_devices()
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                app.recipe_open(Draft::blank(route), false);
            }
        });
        recipes.on_close({
            let app = self.clone();
            move || {
                *app.auto.draft.borrow_mut() = None;
                *app.auto.stored.borrow_mut() = None;
                app.auto.error.borrow_mut().clear();
                app.recipes_publish();
            }
        });
        recipes.on_edited({
            let app = self.clone();
            move || {
                if let Some(ui) = app.ui.upgrade() {
                    let name = ui.global::<Recipes>().get_name().to_string();
                    app.recipe_edit(|draft| draft.name = name);
                }
            }
        });
        recipes.on_pick_target({
            let app = self.clone();
            move |index| {
                let route = app
                    .auto
                    .recipe_routes
                    .borrow()
                    .get(index.max(0) as usize)
                    .cloned();
                if let Some(route) = route {
                    app.recipe_edit(|draft| draft.route_id = route);
                }
            }
        });
        recipes.on_set_install({
            let app = self.clone();
            move |on| app.recipe_edit(|draft| draft.install = on)
        });
        recipes.on_set_shell({
            let app = self.clone();
            move |on| app.recipe_edit(|draft| draft.shell = on)
        });
        recipes.on_step_kind({
            let app = self.clone();
            move |step, kind| {
                let Some((kind, _)) = KINDS.get(kind.max(0) as usize) else {
                    return;
                };
                app.recipe_step(step, |draft| {
                    if draft.kind != *kind {
                        *draft = StepDraft::new(kind);
                    }
                });
            }
        });
        recipes.on_step_text({
            let app = self.clone();
            move |step, text| app.recipe_step(step, |draft| draft.text = text.to_string())
        });
        recipes.on_step_seconds({
            let app = self.clone();
            move |step, text| app.recipe_step(step, |draft| draft.seconds = text.to_string())
        });
        recipes.on_step_mode({
            let app = self.clone();
            move |step, mode| app.recipe_step(step, |draft| draft.mode = mode.to_string())
        });
        recipes.on_step_launch({
            let app = self.clone();
            move |step, on| app.recipe_step(step, |draft| draft.launch = on)
        });
        recipes.on_step_browse({
            let app = self.clone();
            move |step| app.recipe_step_browse(step.max(0) as usize)
        });
        recipes.on_step_move({
            let app = self.clone();
            move |step, by| {
                app.recipe_edit(|draft| {
                    let from = step.max(0) as usize;
                    let to = from as i64 + by as i64;
                    if from < draft.steps.len() && to >= 0 && (to as usize) < draft.steps.len() {
                        draft.steps.swap(from, to as usize);
                    }
                })
            }
        });
        recipes.on_step_remove({
            let app = self.clone();
            move |step| {
                app.recipe_edit(|draft| {
                    let index = step.max(0) as usize;
                    if draft.steps.len() > 1 && index < draft.steps.len() {
                        draft.steps.remove(index);
                    }
                })
            }
        });
        recipes.on_step_add({
            let app = self.clone();
            move || {
                app.recipe_edit(|draft| {
                    if draft.steps.len() < MAX_STEPS {
                        draft.steps.push(StepDraft::new("wait"));
                    }
                })
            }
        });
        recipes.on_save({
            let app = self.clone();
            move || {
                app.recipe_save();
            }
        });
        recipes.on_remove({
            let app = self.clone();
            move || app.recipe_remove_ask()
        });
        recipes.on_run({
            let app = self.clone();
            move || app.recipe_run_ask()
        });
    }

    /// The Automations section came on screen: read what it shows.
    pub fn automations_opened(self: &Rc<Self>) {
        if !self.auto.flutter_asked.replace(true) {
            self.flutter_lookup();
        }
        self.automations_reload();
    }

    fn flutter_lookup(self: &Rc<Self>) {
        let Some(local) = self.inv.local_host() else {
            self.auto.flutter_asked.set(false);
            return;
        };
        let backend = self.backend.clone();
        self.run(
            async move { backend.flutter_info(local.id).await },
            |app, result| {
                *app.auto.flutter.borrow_mut() = Some(result.map_err(|error| error.message));
                app.deploy_publish();
            },
        );
    }

    /// Read recipes and history again and redraw.
    pub fn automations_reload(self: &Rc<Self>) {
        match (self.backend.recipes_list(), self.backend.flow_history()) {
            (Ok(recipes), Ok(history)) => {
                *self.auto.recipes.borrow_mut() = recipes;
                *self.auto.history.borrow_mut() = history;
            }
            (Err(error), _) | (_, Err(error)) => *self.auto.error.borrow_mut() = friendly(&error),
        }
        self.deploy_publish();
        self.recipes_publish();
    }

    /// Phones came or went: the target pickers and the reasons follow.
    pub fn automations_inventory_changed(self: &Rc<Self>) {
        self.deploy_publish();
        self.recipes_publish();
    }
}
