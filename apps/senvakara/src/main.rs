//! Senvakara desktop app, native Slint UI (ADR-023).
//!
//! Home (the inventory and its sections) plus session tabs. A tab holds one session, a
//! workspace of up to four sessions, or a run. Every session view takes its data as a struct
//! and names its session in callbacks, so the same view can be on screen more than once.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod automation;
mod confirm;
mod desktop;
mod device;
mod files;
mod hostform;
mod input;
mod inventory;
mod jobs;
mod logcat;
mod mirror;
mod palette;
mod pane;
mod platform;
mod present;
mod run;
mod settings;
mod state;
mod tabs;
mod term;
mod wireless;
mod workspace;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

pub use crate::confirm::{Pending, Question};
pub use crate::pane::{Pane, Place};
use crate::platform::{select_backend, wsl_distros};
use crate::state::{app, notifier, App, APP};

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    let paths = senvakara_app::AppPaths::resolve()?;
    // One instance per data folder: a second one (this app or the old Tauri one) would reset
    // device states under the first.
    let _instance = match senvakara_app::single_instance(&paths) {
        Ok(guard) => Some(guard),
        Err(senvakara_app::InstanceError::AlreadyRunning) => {
            eprintln!("Senvakara is already running.");
            return Ok(());
        }
        Err(_) => None,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let backend = senvakara_app::App::start(paths, runtime.handle().clone())?;

    select_backend()?;
    let ui = MainWindow::new()?;
    let app = Rc::new(App {
        backend,
        inv: inventory::State::default(),
        hosts: hostform::State::default(),
        dev: device::State::default(),
        jobs: jobs::State::default(),
        wifi: wireless::State::default(),
        ws: workspace::State::default(),
        auto: automation::State::default(),
        settings: settings::State::default(),
        palette: palette::State::default(),
        ui: ui.as_weak(),
        tabs: Rc::new(VecModel::default()),
        sessions: RefCell::new(Vec::new()),
        active: Cell::new(-1),
        next_id: Cell::new(1),
        grid: Cell::new((100, 30)),
        focus_term: Cell::new(0),
        pending: RefCell::new(None),
    });
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));

    let shell = ui.global::<Shell>();
    shell.set_tabs(app.tabs.clone().into());
    shell.on_new_session({
        let app = app.clone();
        move || app.open_shell("powershell", "")
    });

    app.init_inventory(&ui);
    app.init_device_tools(&ui);
    app.init_jobs(&ui);
    app.init_wireless(&ui);
    app.init_host_form(&ui);
    app.init_files(&ui);
    app.init_workspaces(&ui);
    app.init_automations(&ui);
    app.init_run(&ui);
    app.init_settings(&ui);
    app.init_palette(&ui);
    ui.global::<Nav>().on_opened({
        let app = app.clone();
        move |section| app.section_opened(&section)
    });

    ui.global::<Confirm>().on_answered({
        let app = app.clone();
        move |accepted| app.answer(accepted)
    });
    shell.on_select_tab({
        let app = app.clone();
        move |index| app.select(index)
    });
    shell.on_close_tab({
        let app = app.clone();
        move |index| app.request_close(index)
    });
    shell.on_shortcut({
        let app = app.clone();
        move |text, control, shift| app.shortcut(&text, control, shift)
    });

    let view = ui.global::<Term>();
    input::wire_terminal(&view, &app);
    let screen = ui.global::<Mirror>();
    input::wire_mirror(&screen, &app);
    let remote = ui.global::<Desktop>();
    input::wire_desktop(&remote, &app);
    let log = ui.global::<Logcat>();
    input::wire_logcat(&log, &app);

    // Benchmarks and UI tests need a known window size: SENVAKARA_WINDOW_SIZE=936x776.
    if let Some((width, height)) = std::env::var("SENVAKARA_WINDOW_SIZE")
        .ok()
        .and_then(|size| {
            size.split_once('x')
                .map(|(w, h)| (w.parse::<f32>(), h.parse::<f32>()))
        })
        .and_then(|(w, h)| Some((w.ok()?, h.ok()?)))
    {
        ui.window().set_size(slint::LogicalSize::new(width, height));
    }

    ui.run()?;

    // Close every session before the UI goes away so no shell is left behind.
    app.sessions.borrow_mut().clear();
    // A desktop session stops its x11vnc on the host before the app goes away.
    desktop::wait_closed(std::time::Duration::from_secs(4));
    APP.with(|slot| slot.borrow_mut().take());
    Ok(())
}
