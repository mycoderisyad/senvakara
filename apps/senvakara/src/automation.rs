//! Automations: the Flutter build-and-deploy form, saved recipes and the run history. This
//! module checks what the user typed, words every reason, and starts runs; a run itself is a
//! [`crate::run::RunPane`] in a tab of its own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_app::android_tools::valid_package;
use senvakara_app::{
    CoreError, ErrorCode, FlowSpec, FlutterInfo, RecipePermissions, RecipeRecord, RecipeSave,
    RecipeStep, RunRecord,
};
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::inventory::{fact, friendly, friendly_with, transport_label};
use crate::run::{Job, RunPane};
use crate::{
    App, Deploy, KeyValue, Pane, Pending, Question, RecipeItem, Recipes, RunRow, StepItem, TabInfo,
};

type DeviceView = senvakara_app::android::DeviceView;

pub const MAX_STEPS: usize = 20;
const NAME_LIMIT: usize = 80;

/// The step kinds in the order of the picker: (stored kind, label).
pub const KINDS: [(&str, &str); 7] = [
    ("install_apk", "Install APK"),
    ("uninstall_package", "Uninstall app"),
    ("launch_app", "Launch app"),
    ("wait", "Wait"),
    ("device_shell", "Shell command"),
    ("screenshot", "Screenshot"),
    ("flutter_deploy", "Flutter build and install"),
];

/// What the six steps of a build-and-deploy run are called.
const DEPLOY_STEPS: [&str; 6] = [
    "Check the SDK, the project and the phone",
    "Build the APK",
    "Find and fingerprint the APK",
    "Install on the phone",
    "Launch the app",
    "Read the app log for 15 seconds",
];

// ------------------------------------------------------------------ model

/// One recipe step as the editor holds it. Only the fields of its kind are used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepDraft {
    pub kind: String,
    /// The path, package, command, folder or project.
    pub text: String,
    pub seconds: String,
    pub mode: String,
    pub launch: bool,
}

impl StepDraft {
    /// A fresh step of `kind`, with the defaults v0.1.0 used.
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
            text: String::new(),
            seconds: "3".into(),
            mode: "release".into(),
            launch: true,
        }
    }
}

/// The parameter that holds a step's text.
fn text_key(kind: &str) -> &'static str {
    match kind {
        "install_apk" => "path",
        "uninstall_package" | "launch_app" => "package",
        "device_shell" => "command",
        "screenshot" => "dir",
        "flutter_deploy" => "project_dir",
        _ => "",
    }
}

/// Read a stored step into the editor. Missing or odd parameters become empty fields, which
/// the checks then point at; nothing is refused here.
pub fn step_from(step: &RecipeStep) -> StepDraft {
    let params = &step.params;
    let text = params
        .get(text_key(&step.kind))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let seconds = match params.get("seconds") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    StepDraft {
        kind: step.kind.clone(),
        text,
        seconds,
        mode: params
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("release")
            .to_string(),
        launch: params
            .get("launch")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// The stored form of a step: the parameters v0.1.0 wrote for its kind.
pub fn step_to(step: &StepDraft) -> RecipeStep {
    let text = step.text.trim();
    let params = match step.kind.as_str() {
        "wait" => json!({ "seconds": step.seconds.trim().parse::<u64>().unwrap_or(0) }),
        "flutter_deploy" => json!({
            "project_dir": text,
            "mode": step.mode,
            "launch": step.launch,
        }),
        kind => json!({ text_key(kind): text }),
    };
    RecipeStep {
        kind: step.kind.clone(),
        params,
    }
}

/// What is wrong with a step, in words, or "" when the backend will accept it.
pub fn step_problem(step: &StepDraft) -> String {
    let text = step.text.trim();
    match step.kind.as_str() {
        "install_apk" if text.is_empty() => "Choose the APK file to install.".into(),
        "install_apk" if !text.to_ascii_lowercase().ends_with(".apk") => {
            "The file must be an .apk file.".into()
        }
        "uninstall_package" | "launch_app" if text.is_empty() => {
            "Type the package name of the app, for example com.example.app.".into()
        }
        "uninstall_package" | "launch_app" if !valid_package(text) => {
            "That is not a package name. It looks like com.example.app: letters, digits and underscores, with dots between the parts.".into()
        }
        "wait" => match step.seconds.trim().parse::<u64>() {
            Ok(seconds) if (1..=300).contains(&seconds) => String::new(),
            _ => "Type a whole number of seconds from 1 to 300.".into(),
        },
        "device_shell" if text.is_empty() => "Type the command to run on the phone.".into(),
        "device_shell" if text.len() > 4096 => {
            "The command is too long (4096 characters at most).".into()
        }
        "screenshot" if text.is_empty() => "Choose the folder to save the picture in.".into(),
        "flutter_deploy" if text.is_empty() => "Choose the Flutter project folder.".into(),
        "flutter_deploy" if !["debug", "release"].contains(&step.mode.as_str()) => {
            "Choose release or debug.".into()
        }
        kind if !KINDS.iter().any(|(known, _)| *known == kind) => {
            "This build does not know this kind of step. Choose another kind.".into()
        }
        _ => String::new(),
    }
}

/// One line saying what a step does, for the list, the confirmation and the run tab.
pub fn describe_step(step: &StepDraft) -> String {
    let text = step.text.trim();
    let or = |missing: &str| {
        if text.is_empty() {
            missing.to_string()
        } else {
            text.to_string()
        }
    };
    match step.kind.as_str() {
        "install_apk" => format!("Install {}", or("(no file)")),
        "uninstall_package" => format!("Uninstall {}", or("(no package)")),
        "launch_app" => format!("Launch {}", or("(no package)")),
        "wait" => format!("Wait {} s", step.seconds.trim()),
        "device_shell" => format!("Run on the phone: {}", or("(no command)")),
        "screenshot" => format!("Save a screenshot in {}", or("(no folder)")),
        "flutter_deploy" => format!(
            "Build {} ({}) and install it{}",
            or("(no project)"),
            step.mode,
            if step.launch { ", then launch it" } else { "" }
        ),
        other => format!("Unknown step ({other})"),
    }
}

/// Which permission a kind of step needs: "install", "shell" or "".
pub fn needs(kind: &str) -> &'static str {
    match kind {
        "install_apk" | "uninstall_package" | "flutter_deploy" => "install",
        "device_shell" => "shell",
        _ => "",
    }
}

/// The first step that needs a permission the recipe does not have, as a warning.
pub fn permission_warning(steps: &[StepDraft], install: bool, shell: bool) -> String {
    steps
        .iter()
        .enumerate()
        .find_map(|(index, step)| {
            let (missing, permission) = match needs(&step.kind) {
                "install" => (!install, "Install or uninstall apps"),
                "shell" => (!shell, "Run shell commands on the phone"),
                _ => (false, ""),
            };
            let label = KINDS
                .iter()
                .find(|(kind, _)| *kind == step.kind)
                .map(|(_, label)| *label)
                .unwrap_or("This step");
            missing.then(|| {
                format!(
                    "Step {} ({label}) needs the permission \"{permission}\". Tick it above to allow the run.",
                    index + 1
                )
            })
        })
        .unwrap_or_default()
}

pub fn name_problem(name: &str) -> &'static str {
    let name = name.trim();
    if name.is_empty() {
        "Give the recipe a name."
    } else if name.chars().count() > NAME_LIMIT {
        "The name can be at most 80 characters."
    } else {
        ""
    }
}

/// A recipe as the editor holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub id: Option<String>,
    pub name: String,
    /// "" until a phone is chosen.
    pub route_id: String,
    pub steps: Vec<StepDraft>,
    pub install: bool,
    pub shell: bool,
}

impl Draft {
    pub fn blank(route_id: String) -> Self {
        Self {
            id: None,
            name: "New recipe".into(),
            route_id,
            steps: vec![StepDraft::new("launch_app")],
            install: false,
            shell: false,
        }
    }

    pub fn from_record(record: &RecipeRecord) -> Self {
        Self {
            id: Some(record.id.clone()),
            name: record.name.clone(),
            route_id: record.target_route_id.clone().unwrap_or_default(),
            steps: record.steps.iter().map(step_from).collect(),
            install: record.permissions.install,
            shell: record.permissions.shell,
        }
    }

    pub fn to_save(&self) -> RecipeSave {
        RecipeSave {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            target_route_id: (!self.route_id.is_empty()).then(|| self.route_id.clone()),
            steps: self.steps.iter().map(step_to).collect(),
            permissions: RecipePermissions {
                install: self.install,
                shell: self.shell,
            },
        }
    }

    /// Why the recipe cannot be saved as it is, or "".
    pub fn save_problem(&self) -> &'static str {
        let name = name_problem(&self.name);
        if !name.is_empty() {
            name
        } else if self.steps.iter().any(|step| !step_problem(step).is_empty()) {
            "Fix the steps marked in red first."
        } else {
            ""
        }
    }
}

/// Turn a stored raw error ("InvalidInput: step 2: ...", "HOST_OFFLINE: ...") into the words
/// the rest of the app uses. Text that is not such an error is returned as it is.
pub fn friendly_raw(text: &str) -> String {
    const CODES: [(&str, &str, ErrorCode); 12] = [
        (
            "Unauthenticated",
            "UNAUTHENTICATED",
            ErrorCode::Unauthenticated,
        ),
        ("Unauthorized", "UNAUTHORIZED", ErrorCode::Unauthorized),
        ("HostOffline", "HOST_OFFLINE", ErrorCode::HostOffline),
        ("DeviceOffline", "DEVICE_OFFLINE", ErrorCode::DeviceOffline),
        (
            "AdbUnauthorized",
            "ADB_UNAUTHORIZED",
            ErrorCode::AdbUnauthorized,
        ),
        ("RouteStale", "ROUTE_STALE", ErrorCode::RouteStale),
        (
            "UnsupportedCapability",
            "UNSUPPORTED_CAPABILITY",
            ErrorCode::UnsupportedCapability,
        ),
        ("Timeout", "TIMEOUT", ErrorCode::Timeout),
        ("Cancelled", "CANCELLED", ErrorCode::Cancelled),
        ("Conflict", "CONFLICT", ErrorCode::Conflict),
        ("InvalidInput", "INVALID_INPUT", ErrorCode::InvalidInput),
        ("InternalError", "INTERNAL_ERROR", ErrorCode::InternalError),
    ];
    let text = text.trim();
    let Some((code, message)) = text.split_once(':') else {
        return text.to_string();
    };
    match CODES
        .iter()
        .find(|(camel, upper, _)| code == *camel || code == *upper)
    {
        Some((_, _, code)) => friendly_with(
            &CoreError::new(*code, message.trim()),
            "It is already running or was changed in the meantime.",
        ),
        None => text.to_string(),
    }
}

/// "2026-11-22 14:05" in local time, from the UTC time the database stores.
pub fn when_text(created_at: &str) -> String {
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%dT%H:%M:%S%.fZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S"));
    match parsed {
        Ok(time) => time
            .and_utc()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => created_at.to_string(),
    }
}

/// (tone, word) for the status mark of a run.
pub fn run_state(status: &str) -> (&'static str, &'static str) {
    match status {
        "succeeded" => ("live", "Succeeded"),
        "running" | "pending" => ("busy", "Running"),
        "cancelled" => ("idle", "Cancelled"),
        _ => ("attention", "Failed"),
    }
}

/// One history row. `what` is the summary field of the middle column ("mode" or "recipe").
pub fn run_row(run: &RunRecord, what: &str) -> RunRow {
    let field = |key: &str| {
        run.summary
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let (tone, word) = run_state(&run.status);
    RunRow {
        when: when_text(&run.created_at).into(),
        // "Name (host · serial)" as stored; the dot is spelled out as a comma.
        target: field("target").replace(" · ", ", ").into(),
        what: field(what).into(),
        tone: tone.into(),
        state_text: word.into(),
        result: friendly_raw(&field("result")).into(),
        duration: run
            .summary
            .get("seconds")
            .and_then(Value::as_u64)
            .map(|seconds| format!("{seconds} s"))
            .unwrap_or_default()
            .into(),
    }
}

/// Why "Build and deploy" cannot start, or "". `sdk` is None while the SDK is being looked
/// for and `Some(false)` when it was not found.
pub fn deploy_problem(
    sdk: Option<bool>,
    project: &str,
    has_pubspec: bool,
    has_target: bool,
) -> &'static str {
    match sdk {
        None => "Looking for the Flutter SDK.",
        Some(false) => "The Flutter SDK was not found on this PC.",
        Some(true) if project.trim().is_empty() => "Choose the project folder.",
        Some(true) if !has_pubspec => {
            "That folder has no pubspec.yaml, so it is not a Flutter project."
        }
        Some(true) if !has_target => "No phone is connected and authorized.",
        Some(true) => "",
    }
}

fn device_label(device: &DeviceView) -> String {
    format!(
        "{} ({} > {})",
        device.record.display_name,
        device.record.host_name,
        transport_label(&device.record.connection_type)
    )
}

/// Host, route and serial of the phone a run will touch, for a confirmation.
fn target_facts(device: &DeviceView) -> Vec<KeyValue> {
    let record = &device.record;
    vec![
        fact("Phone", record.display_name.as_str(), false),
        fact(
            "Route",
            format!(
                "{} over {}",
                record.host_name,
                transport_label(&record.connection_type)
            ),
            false,
        ),
        fact("Serial", record.serial.as_str(), true),
    ]
}

fn folder_name(path: &str) -> String {
    path.trim()
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_string()
}

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

    // ----------------------------------------------------------- deploy

    fn deploy_publish(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        let view = ui.global::<Deploy>();
        let devices: Vec<DeviceView> = self
            .inv
            .devices()
            .into_iter()
            .filter(|device| device.record.state == "available")
            .collect();
        let routes: Vec<String> = devices
            .iter()
            .map(|device| device.record.route_id.clone())
            .collect();
        // Keep the chosen phone while it is there; otherwise the first one.
        let chosen = {
            let mut route = self.auto.deploy_route.borrow_mut();
            if !routes.contains(&route) {
                *route = routes.first().cloned().unwrap_or_default();
            }
            routes.iter().position(|r| *r == *route).unwrap_or(0)
        };
        let labels: Vec<SharedString> = if devices.is_empty() {
            vec!["No phone is connected and authorized".into()]
        } else {
            devices
                .iter()
                .map(|device| device_label(device).into())
                .collect()
        };
        let current: Vec<SharedString> = view.get_targets().iter().collect();
        if current != labels {
            view.set_targets(ModelRc::new(VecModel::from(labels)));
        }
        view.set_target_index(chosen as i32);
        *self.auto.deploy_routes.borrow_mut() = routes;

        let flutter = self.auto.flutter.borrow();
        let (tone, text, detail) = match &*flutter {
            None => (
                "busy",
                "Looking for the Flutter SDK".to_string(),
                String::new(),
            ),
            Some(Ok(info)) => (
                "live",
                // The SDK prints its version, channel and source in one line with bullets
                // between them: the version is the first part.
                match info.version.split('\u{2022}').next().map(str::trim) {
                    Some(version) if !version.is_empty() => version.to_string(),
                    _ => "Flutter SDK found".to_string(),
                },
                info.path.clone(),
            ),
            Some(Err(_)) => (
                "attention",
                "Flutter SDK not found".to_string(),
                "Install Flutter and put it on PATH, or set FLUTTER_ROOT, then restart Senvakara."
                    .to_string(),
            ),
        };
        view.set_sdk_tone(tone.into());
        view.set_sdk_text(text.into());
        view.set_sdk_detail(detail.into());
        view.set_build_host(
            self.inv
                .local_host()
                .map(|host| host.display_name)
                .unwrap_or_else(|| "this PC".into())
                .into(),
        );
        view.set_mode(self.auto.mode.borrow().as_str().into());
        view.set_launch(self.auto.launch.get());
        view.set_logcat(self.auto.logcat.get());
        let project = view.get_project().to_string();
        let has_pubspec = std::path::Path::new(project.trim())
            .join("pubspec.yaml")
            .is_file();
        view.set_reason(
            deploy_problem(
                flutter.as_ref().map(Result::is_ok),
                &project,
                has_pubspec,
                !devices.is_empty(),
            )
            .into(),
        );
        let rows: Vec<RunRow> = self
            .auto
            .history
            .borrow()
            .iter()
            .filter(|run| run.recipe_id.is_none())
            .map(|run| run_row(run, "mode"))
            .collect();
        let current: Vec<RunRow> = view.get_history().iter().collect();
        if current != rows {
            view.set_history(ModelRc::new(VecModel::from(rows)));
        }
    }

    /// Everything a deploy needs, read from the form as it is now.
    fn deploy_spec(&self) -> Option<(FlowSpec, DeviceView, String)> {
        let ui = self.ui.upgrade()?;
        let local = self.inv.local_host()?;
        let device = self.inv.device(&self.auto.deploy_route.borrow())?;
        if device.record.state != "available" {
            return None;
        }
        let launch = self.auto.launch.get();
        let spec = FlowSpec {
            build_host_id: local.id,
            project_dir: ui.global::<Deploy>().get_project().trim().to_string(),
            mode: self.auto.mode.borrow().clone(),
            route_id: device.record.route_id.clone(),
            expected_serial: device.record.serial.clone(),
            launch,
            logcat: launch && self.auto.logcat.get(),
        };
        Some((spec, device, local.display_name))
    }

    fn deploy_ask(self: &Rc<Self>) {
        let Some(ui) = self.ui.upgrade() else { return };
        if !ui.global::<Deploy>().get_reason().is_empty() {
            return;
        }
        let Some((spec, device, build_host)) = self.deploy_spec() else {
            return;
        };
        let mut facts = vec![
            fact("Project", spec.project_dir.as_str(), true),
            fact("Mode", spec.mode.as_str(), false),
            fact("Build host", build_host, false),
        ];
        facts.extend(target_facts(&device));
        let after = match (spec.launch, spec.logcat) {
            (true, true) => " Then it is launched and its log is read for 15 seconds.",
            (true, false) => " Then it is launched.",
            _ => "",
        };
        self.ask_with(
            Question {
                title: "Build and install this app?".into(),
                body: format!(
                    "The app is built and installed on the phone, replacing an older version of the same package.{after}"
                ),
                confirm: "Build and deploy".into(),
                danger: false,
            },
            facts,
            "",
            Pending::Deploy,
        );
    }

    /// The user confirmed the deploy: open its run tab.
    pub fn deploy_start(self: &Rc<Self>) {
        let Some((spec, device, _)) = self.deploy_spec() else {
            return;
        };
        let title = format!("Deploy {}", folder_name(&spec.project_dir));
        let labels = DEPLOY_STEPS.iter().map(|label| label.to_string()).collect();
        self.open_run(title, &device, labels, Job::Deploy(spec));
    }

    fn open_run(
        self: &Rc<Self>,
        title: String,
        device: &DeviceView,
        labels: Vec<String>,
        job: Job,
    ) {
        let id = self.take_id();
        let record = &device.record;
        let hue = self.inv.hue(&record.host_id);
        let target = format!(
            "{}, through {} over {}",
            record.display_name,
            record.host_name,
            transport_label(&record.connection_type)
        );
        let pane = RunPane::start(
            id,
            self.backend.clone(),
            title.clone(),
            target,
            hue,
            labels,
            job,
            crate::notifier(id),
        );
        self.sessions.borrow_mut().push(Pane::Run(pane));
        self.tabs.push(TabInfo {
            id,
            kind: "run".into(),
            title: title.into(),
            detail: SharedString::new(),
            hue,
            exited: false,
        });
        self.select(self.tabs.row_count() as i32 - 1);
    }

    // ---------------------------------------------------------- recipes

    fn recipe_open(self: &Rc<Self>, draft: Draft, stored: bool) {
        *self.auto.stored.borrow_mut() = stored.then(|| draft.clone());
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Recipes>().set_name(draft.name.as_str().into());
        }
        *self.auto.draft.borrow_mut() = Some(draft);
        self.auto.error.borrow_mut().clear();
        self.recipes_publish();
    }

    /// Change the open recipe and redraw.
    fn recipe_edit(self: &Rc<Self>, change: impl FnOnce(&mut Draft)) {
        if let Some(draft) = self.auto.draft.borrow_mut().as_mut() {
            change(draft);
        }
        self.recipes_publish();
    }

    fn recipe_step(self: &Rc<Self>, step: i32, change: impl FnOnce(&mut StepDraft)) {
        self.recipe_edit(|draft| {
            if let Some(step) = draft.steps.get_mut(step.max(0) as usize) {
                change(step);
            }
        });
    }

    fn recipe_step_browse(self: &Rc<Self>, index: usize) {
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

    fn recipes_publish(self: &Rc<Self>) {
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
    fn recipe_save(self: &Rc<Self>) -> Option<String> {
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

    fn recipe_remove_ask(self: &Rc<Self>) {
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

    fn recipe_run_ask(self: &Rc<Self>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: &str, text: &str) -> StepDraft {
        StepDraft {
            text: text.into(),
            ..StepDraft::new(kind)
        }
    }

    #[test]
    fn steps_are_stored_with_the_v010_parameters() {
        assert_eq!(
            step_to(&step("install_apk", " C:\\a\\app.apk ")).params,
            json!({"path": "C:\\a\\app.apk"})
        );
        assert_eq!(
            step_to(&step("uninstall_package", "dev.x.app")).params,
            json!({"package": "dev.x.app"})
        );
        assert_eq!(
            step_to(&step("launch_app", "dev.x.app")).params,
            json!({"package": "dev.x.app"})
        );
        assert_eq!(
            step_to(&StepDraft {
                seconds: " 12 ".into(),
                ..StepDraft::new("wait")
            })
            .params,
            json!({"seconds": 12})
        );
        assert_eq!(
            step_to(&step("device_shell", "id")).params,
            json!({"command": "id"})
        );
        assert_eq!(
            step_to(&step("screenshot", "D:\\shots")).params,
            json!({"dir": "D:\\shots"})
        );
        assert_eq!(
            step_to(&StepDraft {
                text: "D:\\p".into(),
                mode: "debug".into(),
                launch: false,
                ..StepDraft::new("flutter_deploy")
            })
            .params,
            json!({"project_dir": "D:\\p", "mode": "debug", "launch": false})
        );
    }

    #[test]
    fn stored_steps_round_trip_through_the_editor() {
        let stored = [
            ("install_apk", json!({"path": "C:\\a.apk"})),
            ("uninstall_package", json!({"package": "dev.x.app"})),
            ("launch_app", json!({"package": "dev.x.app"})),
            ("wait", json!({"seconds": 3})),
            ("device_shell", json!({"command": "getprop"})),
            ("screenshot", json!({"dir": "D:\\shots"})),
            (
                "flutter_deploy",
                json!({"project_dir": "D:\\p", "mode": "release", "launch": true}),
            ),
        ];
        for (kind, params) in stored {
            let record = RecipeStep {
                kind: kind.into(),
                params,
            };
            let draft = step_from(&record);
            assert_eq!(step_problem(&draft), "", "{kind}");
            assert_eq!(step_to(&draft), record);
        }
    }

    #[test]
    fn broken_stored_steps_open_with_a_marked_problem() {
        for (kind, params) in [
            ("wait", json!({})),
            ("wait", json!({"seconds": "soon"})),
            ("launch_app", json!(null)),
            ("install_apk", json!({"path": 7})),
            ("teleport", json!({"x": 1})),
        ] {
            let draft = step_from(&RecipeStep {
                kind: kind.into(),
                params,
            });
            assert_ne!(step_problem(&draft), "", "{kind}");
        }
    }

    #[test]
    fn step_checks_match_the_backend_rules() {
        assert_eq!(step_problem(&step("install_apk", "C:\\a\\App.APK")), "");
        assert!(step_problem(&step("install_apk", "C:\\a.exe")).contains(".apk"));
        assert!(step_problem(&step("install_apk", " ")).contains("Choose"));
        assert_eq!(step_problem(&step("launch_app", "dev.x.app")), "");
        assert!(step_problem(&step("launch_app", "x; reboot")).contains("package name"));
        assert!(step_problem(&step("uninstall_package", "")).contains("package name"));
        for (seconds, ok) in [
            ("1", true),
            ("300", true),
            ("0", false),
            ("301", false),
            ("2.5", false),
            ("", false),
        ] {
            let draft = StepDraft {
                seconds: seconds.into(),
                ..StepDraft::new("wait")
            };
            assert_eq!(step_problem(&draft).is_empty(), ok, "{seconds}");
        }
        assert!(step_problem(&step("device_shell", "")).contains("command"));
        assert!(step_problem(&step("device_shell", &"x".repeat(4097))).contains("too long"));
        assert!(step_problem(&step("screenshot", "")).contains("folder"));
        assert!(step_problem(&step("flutter_deploy", "")).contains("project"));
        let odd_mode = StepDraft {
            text: "D:\\p".into(),
            mode: "profile".into(),
            ..StepDraft::new("flutter_deploy")
        };
        assert!(step_problem(&odd_mode).contains("release or debug"));
    }

    #[test]
    fn what_the_editor_accepts_passes_the_backend_check() {
        let steps = [
            step("install_apk", "C:\\a.apk"),
            step("launch_app", "dev.x.app"),
            StepDraft::new("wait"),
            step("device_shell", "id"),
            step("screenshot", "D:\\shots"),
            step("flutter_deploy", "D:\\p"),
        ];
        assert!(steps.iter().all(|s| step_problem(s).is_empty()));
        let stored: Vec<RecipeStep> = steps.iter().map(step_to).collect();
        let all = RecipePermissions {
            install: true,
            shell: true,
        };
        assert!(senvakara_app::recipes::check_plan(&stored, &all).is_ok());
    }

    #[test]
    fn missing_permissions_are_named() {
        let steps = vec![
            StepDraft::new("wait"),
            step("device_shell", "id"),
            step("install_apk", "a.apk"),
        ];
        assert_eq!(permission_warning(&steps, true, true), "");
        let shell = permission_warning(&steps, true, false);
        assert!(shell.starts_with("Step 2 (Shell command)"), "{shell}");
        assert!(shell.contains("Run shell commands on the phone"));
        let install = permission_warning(&steps, false, true);
        assert!(install.starts_with("Step 3 (Install APK)"), "{install}");
        assert_eq!(permission_warning(&steps[..1], false, false), "");
    }

    #[test]
    fn a_recipe_needs_a_name_and_sound_steps() {
        let mut draft = Draft::blank("route_1".into());
        // The blank recipe has a launch step without a package.
        assert!(draft.save_problem().contains("steps"));
        draft.steps = vec![StepDraft::new("wait")];
        assert_eq!(draft.save_problem(), "");
        draft.name = "  ".into();
        assert!(draft.save_problem().contains("name"));
        draft.name = "x".repeat(81);
        assert!(draft.save_problem().contains("80"));
        draft.name = "é".repeat(80);
        assert_eq!(draft.save_problem(), "");
    }

    #[test]
    fn a_draft_is_saved_as_the_backend_expects() {
        let mut draft = Draft::blank(String::new());
        draft.name = " Smoke test ".into();
        draft.steps = vec![StepDraft::new("wait")];
        let save = draft.to_save();
        assert_eq!(save.id, None);
        assert_eq!(save.name, "Smoke test");
        assert_eq!(save.target_route_id, None);
        assert_eq!(save.steps[0].params, json!({"seconds": 3}));
        draft.route_id = "route_1".into();
        assert_eq!(draft.to_save().target_route_id.as_deref(), Some("route_1"));
    }

    #[test]
    fn steps_are_described_in_words() {
        assert_eq!(describe_step(&StepDraft::new("wait")), "Wait 3 s");
        assert_eq!(
            describe_step(&step("launch_app", "dev.x.app")),
            "Launch dev.x.app"
        );
        assert_eq!(describe_step(&step("install_apk", "")), "Install (no file)");
        assert_eq!(
            describe_step(&step("flutter_deploy", "D:\\p")),
            "Build D:\\p (release) and install it, then launch it"
        );
    }

    #[test]
    fn stored_raw_errors_are_worded_for_people() {
        let worded = friendly_raw("InvalidInput: step 1: invalid package name");
        assert!(worded.starts_with("Something in the request is not valid."));
        assert!(worded.contains("step 1: invalid package name"));
        assert!(friendly_raw("DEVICE_OFFLINE: gone").starts_with("The phone is not connected"));
        assert!(friendly_raw("AdbUnauthorized: ").starts_with("The phone has not allowed"));
        // Not an error code: left alone.
        assert_eq!(
            friendly_raw("Installed and launched: dev.x.app"),
            "Installed and launched: dev.x.app"
        );
        assert_eq!(friendly_raw("3 steps done"), "3 steps done");
        assert_eq!(friendly_raw(""), "");
    }

    #[test]
    fn history_rows_read_the_stored_summary() {
        let run = RunRecord {
            id: "run_1".into(),
            recipe_id: Some("rec_1".into()),
            status: "failed".into(),
            created_at: "not a time".into(),
            finished_at: None,
            summary: json!({
                "recipe": "Smoke test",
                "target": "Redmi (RAFGT · abc123)",
                "result": "Timeout: build took too long",
                "seconds": 42,
            }),
            steps: Vec::new(),
        };
        let row = run_row(&run, "recipe");
        assert_eq!(row.when, "not a time");
        assert_eq!(row.target, "Redmi (RAFGT, abc123)");
        assert_eq!(row.what, "Smoke test");
        assert_eq!(row.state_text, "Failed");
        assert_eq!(row.tone, "attention");
        assert!(row.result.starts_with("It took too long"));
        assert_eq!(row.duration, "42 s");
        // A run that is still going has no result or duration yet.
        let running = RunRecord {
            status: "running".into(),
            summary: json!({"mode": "debug"}),
            ..run
        };
        let row = run_row(&running, "mode");
        assert_eq!(
            (row.what.as_str(), row.state_text.as_str()),
            ("debug", "Running")
        );
        assert_eq!(row.duration, "");
    }

    #[test]
    fn stored_times_are_shown_as_minutes() {
        let text = when_text("2026-11-22T10:15:30.123Z");
        assert_eq!(text.len(), "2026-11-22 10:15".len());
        assert!(text.starts_with("2026-11-2"));
    }

    #[test]
    fn deploy_is_blocked_until_everything_is_there() {
        assert!(deploy_problem(None, "D:\\p", true, true).contains("Looking"));
        assert!(deploy_problem(Some(false), "D:\\p", true, true).contains("not found"));
        assert!(deploy_problem(Some(true), " ", false, true).contains("project folder"));
        assert!(deploy_problem(Some(true), "D:\\p", false, true).contains("pubspec.yaml"));
        assert!(deploy_problem(Some(true), "D:\\p", true, false).contains("No phone"));
        assert_eq!(deploy_problem(Some(true), "D:\\p", true, true), "");
    }

    #[test]
    fn folder_names_ignore_trailing_separators() {
        assert_eq!(folder_name("D:\\projects\\my_app\\"), "my_app");
        assert_eq!(folder_name("/home/u/app"), "app");
    }
}
