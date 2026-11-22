//! The recipe as the editor holds it: the step kinds, the step and recipe drafts, and their checks.

use senvakara_app::android_tools::valid_package;
use senvakara_app::{RecipePermissions, RecipeRecord, RecipeSave, RecipeStep};
use serde_json::{json, Value};

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

#[cfg(test)]
mod tests;
