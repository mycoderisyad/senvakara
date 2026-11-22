//! Running a recipe: the confirmation, then the run tab.

use std::rc::Rc;

use slint::ComponentHandle;

use crate::inventory::fact;
use crate::run::Job;
use crate::{App, Pending, Question, Recipes};

use super::draft::describe_step;
use super::target::target_facts;

impl App {
    pub(super) fn recipe_run_ask(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !ui.global::<Recipes>().get_run_reason().is_empty() {
            return;
        }
        let Some(draft) = self.auto.draft.borrow().clone() else {
            return;
        };
        let Some(device) = self.inv.device(&draft.route_id) else {
            return;
        };
        let mut facts = target_facts(&device);
        for (index, step) in draft.steps.iter().enumerate() {
            facts.push(fact(
                &format!("Step {}", index + 1),
                describe_step(step),
                false,
            ));
        }
        facts.push(fact(
            "Permissions",
            format!(
                "{}; {}",
                if draft.install {
                    "may install and uninstall apps"
                } else {
                    "no installs"
                },
                if draft.shell {
                    "may run shell commands"
                } else {
                    "no shell commands"
                }
            ),
            false,
        ));
        let dirty = self.auto.stored.borrow().as_ref() != Some(&draft);
        self.ask_with(
            Question {
                title: format!("Run {}?", draft.name.trim()),
                body: format!(
                    "{}The steps run in order on this phone and the run stops at the first failed step.",
                    if dirty {
                        "Your changes are saved first. "
                    } else {
                        ""
                    }
                ),
                confirm: "Run recipe".into(),
                danger: false,
            },
            facts,
            "",
            Pending::RunRecipe,
        );
    }

    /// The user confirmed the run: save the recipe if it changed, then open the run tab.
    pub fn recipe_run(self: &Rc<Self>) {
        let Some(recipe_id) = self.recipe_save() else {
            return;
        };
        let Some(draft) = self.auto.draft.borrow().clone() else {
            return;
        };
        let Some(device) = self.inv.device(&draft.route_id) else {
            return;
        };
        if device.record.state != "available" {
            return;
        }
        let labels = draft.steps.iter().map(describe_step).collect();
        let job = Job::Recipe {
            recipe_id,
            serial: device.record.serial.clone(),
        };
        self.open_run(format!("Run {}", draft.name.trim()), &device, labels, job);
    }
}
