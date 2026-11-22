//! Automations: the Flutter build-and-deploy form, saved recipes and the run history. This
//! module checks what the user typed, words every reason, and starts runs; a run itself is a
//! [`crate::run::RunPane`] in a tab of its own.

mod deploy;
mod draft;
mod history;
mod recipe_run;
mod recipes;
mod section;
mod target;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_app::{FlutterInfo, RecipeRecord, RunRecord};
use slint::VecModel;

use crate::{RecipeItem, StepItem};

use draft::Draft;
pub use history::friendly_raw;

type DeviceView = senvakara_app::android::DeviceView;

// ------------------------------------------------------------------ state

pub struct State {
    /// None until asked; then the SDK or why it was not found.
    flutter: RefCell<Option<Result<FlutterInfo, String>>>,
    flutter_asked: Cell<bool>,
    mode: RefCell<String>,
    launch: Cell<bool>,
    logcat: Cell<bool>,
    /// The phone chosen for a deploy.
    deploy_route: RefCell<String>,
    /// Route ids behind the deploy target picker, in its order.
    deploy_routes: RefCell<Vec<String>>,
    recipes: RefCell<Vec<RecipeRecord>>,
    draft: RefCell<Option<Draft>>,
    /// The open recipe as it is stored; None while it was never saved.
    stored: RefCell<Option<Draft>>,
    /// Route ids behind the recipe target picker; the first entry is "no phone".
    recipe_routes: RefCell<Vec<String>>,
    history: RefCell<Vec<RunRecord>>,
    error: RefCell<String>,
    steps: Rc<VecModel<StepItem>>,
    items: Rc<VecModel<RecipeItem>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            flutter: RefCell::new(None),
            flutter_asked: Cell::new(false),
            mode: RefCell::new("release".into()),
            launch: Cell::new(true),
            logcat: Cell::new(true),
            deploy_route: RefCell::new(String::new()),
            deploy_routes: RefCell::new(Vec::new()),
            recipes: RefCell::new(Vec::new()),
            draft: RefCell::new(None),
            stored: RefCell::new(None),
            recipe_routes: RefCell::new(Vec::new()),
            history: RefCell::new(Vec::new()),
            error: RefCell::new(String::new()),
            steps: Rc::new(VecModel::default()),
            items: Rc::new(VecModel::default()),
        }
    }
}
