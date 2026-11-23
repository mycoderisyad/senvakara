//! The Flutter build-and-deploy card: its checks, what it shows, and starting a deploy.

use std::rc::Rc;

use senvakara_app::FlowSpec;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::inventory::fact;
use crate::run::Job;
use crate::{App, Deploy, Pending, Question, RunRow};

use super::history::{is_standalone_deploy, run_row};
use super::target::{device_label, target_facts};
use super::DeviceView;

/// What the six steps of a build-and-deploy run are called.
const DEPLOY_STEPS: [&str; 6] = [
    "Check the SDK, the project and the phone",
    "Build the APK",
    "Find and fingerprint the APK",
    "Install on the phone",
    "Launch the app",
    "Read the app log for 15 seconds",
];

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

/// Whether the phone found for a confirmed deploy is still the one that was confirmed.
fn same_target(spec: &FlowSpec, state: &str, serial: &str) -> bool {
    state == "available" && serial == spec.expected_serial
}

/// The Deploy card lists this many runs.
const HISTORY_SHOWN: usize = 50;

fn folder_name(path: &str) -> String {
    path.trim()
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_string()
}

impl App {
    pub(super) fn deploy_publish(self: &Rc<Self>) {
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
            .filter(|run| is_standalone_deploy(run))
            .take(HISTORY_SHOWN)
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

    pub(super) fn deploy_ask(self: &Rc<Self>) {
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
            Pending::Deploy(spec),
        );
    }

    /// The user confirmed the deploy `spec` the dialog described: open its run tab. It runs
    /// for exactly that phone, or not at all when the phone is no longer there as it was.
    pub fn deploy_start(self: &Rc<Self>, spec: FlowSpec) {
        let device = self
            .inv
            .device(&spec.route_id)
            .filter(|device| same_target(&spec, &device.record.state, &device.record.serial));
        let Some(device) = device else {
            // The phone the dialog named went away: show the form as it is now.
            self.deploy_publish();
            return;
        };
        let title = format!("Deploy {}", folder_name(&spec.project_dir));
        let labels = DEPLOY_STEPS.iter().map(|label| label.to_string()).collect();
        self.open_run(title, &device, labels, Job::Deploy(spec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_confirmed_deploy_only_runs_for_the_phone_that_was_confirmed() {
        let spec = FlowSpec {
            build_host_id: "local".into(),
            project_dir: "D:/p".into(),
            mode: "debug".into(),
            route_id: "route-a".into(),
            expected_serial: "AAA".into(),
            launch: false,
            logcat: false,
        };
        assert!(same_target(&spec, "available", "AAA"));
        assert!(!same_target(&spec, "available", "BBB"));
        assert!(!same_target(&spec, "offline", "AAA"));
    }

    #[test]
    fn folder_names_ignore_trailing_separators() {
        assert_eq!(folder_name("D:\\projects\\my_app\\"), "my_app");
        assert_eq!(folder_name("/home/u/app"), "app");
    }
}
