//! Tests for the recipe draft model and its checks.

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
