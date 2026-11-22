//! Tests of the stored form: what v0.1.0 wrote is read, and what is written reads back.

use super::*;

use crate::workspace::availability::blocked;

fn panel(panel_type: &str, target: Value) -> PanelRecord {
    PanelRecord {
        panel_type: panel_type.into(),
        target,
    }
}

fn record(layout: Value, panels: Vec<PanelRecord>) -> WorkspaceRecord {
    WorkspaceRecord {
        id: "ws_1".into(),
        name: "Bench".into(),
        layout,
        panels,
        updated_at: String::new(),
    }
}

/// What `WorkspacesView.tsx` of v0.1.0 stored for one panel of every kind.
fn v010_panels() -> Vec<PanelRecord> {
    vec![
        panel(
            "terminal",
            json!({"title": "PowerShell", "shell": {"kind": "power_shell"}}),
        ),
        panel(
            "terminal",
            json!({"title": "WSL · Ubuntu", "shell": {"kind": "wsl", "distro": "Ubuntu"}}),
        ),
        panel(
            "terminal",
            json!({"title": "tmux · fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": "senvakara-m3x9k2"}}),
        ),
        panel("mirror", json!({"route_id": "route_1"})),
    ]
}

#[test]
fn reads_what_v010_wrote() {
    let (layout, slots) = read_workspace(&record(json!({"layout": "grid"}), v010_panels()));
    assert_eq!(layout, Layout::Grid);
    assert_eq!(
        slots,
        vec![
            Some(Spec::Terminal {
                title: "PowerShell".into(),
                shell: Shell::Windows
            }),
            Some(Spec::Terminal {
                title: "WSL · Ubuntu".into(),
                shell: Shell::Wsl(Some("Ubuntu".into()))
            }),
            Some(Spec::Terminal {
                title: "tmux · fedora".into(),
                shell: Shell::Ssh {
                    host_id: "host_a".into(),
                    tmux: Some("senvakara-m3x9k2".into())
                }
            }),
            Some(Spec::Mirror {
                route_id: "route_1".into()
            }),
        ]
    );
}

#[test]
fn every_panel_kind_round_trips_in_the_v010_shape() {
    let stored = vec![
        panel(
            "terminal",
            json!({"title": "SSH · fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": null}}),
        ),
        panel(
            "terminal",
            json!({"title": "WSL", "shell": {"kind": "wsl", "distro": null}}),
        ),
        panel("logcat", json!({"route_id": "route_1"})),
        panel("desktop", json!({"host_id": "host_a"})),
        panel("files", json!({"host_id": "host_a"})),
        panel("mirror", json!({"route_id": "route_2"})),
    ];
    for record in stored {
        assert_eq!(panel_record(&parse_panel(&record)), record);
    }
}

#[test]
fn a_missing_or_unknown_layout_is_two_columns() {
    for layout in [
        json!({}),
        json!(null),
        json!({"layout": "ring"}),
        json!("x"),
    ] {
        let (parsed, slots) = read_workspace(&record(layout, v010_panels()));
        assert_eq!(parsed, Layout::Columns);
        // Only the first two panels fit, as in v0.1.0.
        assert_eq!(slots.len(), 2);
        assert!(matches!(slots[1], Some(Spec::Terminal { .. })));
    }
}

#[test]
fn broken_panels_become_unavailable_and_are_kept_as_stored() {
    let broken = vec![
        panel("terminal", json!({"title": "x"})),
        panel("terminal", json!({"shell": {"kind": "telnet"}})),
        panel("terminal", json!({"shell": {"kind": "ssh"}})),
        panel("mirror", json!({})),
        panel("files", json!({"host_id": 7})),
        panel("desktop", json!(null)),
        panel("hologram", json!({"x": 1})),
    ];
    for record in &broken {
        let spec = parse_panel(record);
        assert!(matches!(spec, Spec::Unavailable { .. }), "{record:?}");
        assert_eq!(&panel_record(&spec), record);
        assert!(blocked(&spec, &[], &[]).is_some());
    }
    // A panel type the database refuses is not written back.
    let slots: Vec<Option<Spec>> = broken.iter().map(|r| Some(parse_panel(r))).collect();
    let (_, panels) = write_workspace(Layout::Grid, &slots[3..]);
    assert_eq!(panels.len(), 3);
    assert!(panels.iter().all(|p| p.panel_type != "hologram"));
}

#[test]
fn a_terminal_without_a_title_gets_one() {
    let spec = parse_panel(&panel(
        "terminal",
        json!({"shell": {"kind": "power_shell"}}),
    ));
    assert_eq!(
        spec,
        Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows
        }
    );
}

#[test]
fn panels_fill_slots_in_order_without_positions() {
    assert_eq!(
        place_panels(2, &json!({"layout": "grid"}), 4),
        vec![Some(0), Some(1), None, None]
    );
    assert_eq!(place_panels(3, &json!({}), 2), vec![Some(0), Some(1)]);
    assert_eq!(place_panels(0, &json!({}), 1), vec![None]);
}

#[test]
fn stored_positions_put_panels_back_where_they_were() {
    assert_eq!(
        place_panels(3, &json!({"slots": [0, 2, 3]}), 4),
        vec![Some(0), None, Some(1), Some(2)]
    );
    assert_eq!(
        place_panels(2, &json!({"slots": [3, 1]}), 4),
        vec![None, Some(1), None, Some(0)]
    );
}

#[test]
fn bad_positions_fall_back_to_the_free_slots() {
    // Out of range, duplicate, wrong type, too short: nothing is lost that fits.
    assert_eq!(
        place_panels(3, &json!({"slots": [9, 1, 1]}), 4),
        vec![Some(0), Some(1), Some(2), None]
    );
    assert_eq!(
        place_panels(2, &json!({"slots": "x"}), 2),
        vec![Some(0), Some(1)]
    );
    assert_eq!(
        place_panels(3, &json!({"slots": [2]}), 4),
        vec![Some(1), Some(2), Some(0), None]
    );
    assert_eq!(
        place_panels(2, &json!({"slots": [-1, 0.5]}), 2),
        vec![Some(0), Some(1)]
    );
}

#[test]
fn saving_keeps_positions_and_the_v010_layout_key() {
    let slots = vec![
        Some(Spec::Terminal {
            title: "PowerShell".into(),
            shell: Shell::Windows,
        }),
        None,
        Some(Spec::Logcat {
            route_id: "route_1".into(),
        }),
        Some(Spec::Terminal {
            title: "SSH fedora".into(),
            shell: Shell::Ssh {
                host_id: "host_a".into(),
                tmux: None,
            },
        }),
    ];
    let (layout, panels) = write_workspace(Layout::Grid, &slots);
    assert_eq!(layout, json!({"layout": "grid", "slots": [0, 2, 3]}));
    assert_eq!(
        panels[0].target,
        json!({"title": "PowerShell", "shell": {"kind": "power_shell"}})
    );
    assert_eq!(panels[1], panel("logcat", json!({"route_id": "route_1"})));
    assert_eq!(
        panels[2].target,
        json!({"title": "SSH fedora", "shell": {"kind": "ssh", "host_id": "host_a", "tmux": null}})
    );
    // Reading it back gives the same slots.
    let (read_layout, read_slots) = read_workspace(&record(layout, panels));
    assert_eq!(read_layout, Layout::Grid);
    assert_eq!(read_slots, slots);
}

#[test]
fn panels_beyond_the_layout_are_not_saved() {
    let slots = vec![
        Some(Spec::Mirror {
            route_id: "a".into(),
        }),
        Some(Spec::Mirror {
            route_id: "b".into(),
        }),
    ];
    let (layout, panels) = write_workspace(Layout::Single, &slots);
    assert_eq!(layout, json!({"layout": "single", "slots": [0]}));
    assert_eq!(panels.len(), 1);
}
