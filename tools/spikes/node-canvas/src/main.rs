//! Phase 0 spike: slint-node-editor 2.0.0 under Senvakara's Slint pin.
//!
//! Env knobs:
//! - `SPIKE_NODES` (default 50): nodes on an 8-column grid, chained by links.
//! - `SPIKE_ZOOM` (default 1.0): initial zoom.
//! - `SPIKE_ANIM=1`: move every node each 16 ms (worst case: whole graph dragged).
//! - `SPIKE_SNAPSHOT=<file.png>`: render after 1.5 s, save the frame, exit.
//! - `SPIKE_EXIT_AFTER=<secs>`: quit after this many seconds (for measurements).

use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
#[allow(unused_imports)] // the wiring macros expect `LinkPath` in scope
use slint_node_editor::{
    wire_node_editor, wire_selection, GraphLogic, LinkData, LinkPath, MovableNode, NodeEditorSetup,
};
use std::{cell::Cell, collections::HashSet, rc::Rc, time::Duration};

slint::include_modules!();

impl MovableNode for NodeData {
    fn id(&self) -> i32 {
        self.id
    }
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn selected(&self) -> bool {
        self.selected
    }
    fn set_x(&mut self, x: f32) {
        self.x = x;
    }
    fn set_y(&mut self, y: f32) {
        self.y = y;
    }
}

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn remove_where<T: Clone + 'static>(model: &VecModel<T>, remove: impl Fn(&T) -> bool) {
    for index in (0..model.row_count()).rev() {
        if model.row_data(index).is_some_and(|row| remove(&row)) {
            model.remove(index);
        }
    }
}

const CATEGORIES: [&str; 6] = ["Host", "Security", "Database", "App", "Deployment", "Monitoring"];

fn main() -> Result<(), slint::PlatformError> {
    let count: i32 = env("SPIKE_NODES", 50);
    let window = App::new()?;
    window.set_zoom(env("SPIKE_ZOOM", 1.0f32));
    // SPIKE_GRID=<px>: grid spacing; a huge value effectively removes the grid.
    window.set_grid_step(env("SPIKE_GRID", 24.0f32));

    let nodes = Rc::new(VecModel::from(
        (1..=count)
            .map(|id| {
                let i = id - 1;
                NodeData {
                    id,
                    title: SharedString::from(format!("Node {id}")),
                    category: CATEGORIES[i as usize % CATEGORIES.len()].into(),
                    status: "Up to date".into(),
                    x: 40.0 + (i % 8) as f32 * 260.0,
                    y: 40.0 + (i / 8) as f32 * 120.0,
                    selected: false,
                }
            })
            .collect::<Vec<_>>(),
    ));
    // SPIKE_LINKS=0 drops the chain of links (isolates link cost).
    let link_count = if env("SPIKE_LINKS", 1) == 0 { 1 } else { count };
    let links = Rc::new(VecModel::from(
        (1..link_count)
            .map(|id| LinkData {
                id,
                start_pin_id: id * 2 + 1,
                end_pin_id: (id + 1) * 2,
                color: Color::from_rgb_u8(110, 168, 254),
                selected: false,
                line_width: 2.0,
                status: -1,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ));
    window.set_nodes(ModelRc::from(nodes.clone()));
    window.set_links(ModelRc::from(links.clone()));

    let setup = NodeEditorSetup::new({
        let nodes = nodes.clone();
        move |dragged, dx, dy| GraphLogic::commit_drag(&nodes, dragged, dx, dy)
    });
    let controller = setup.controller().clone();
    wire_node_editor!(window, setup);
    wire_selection!(window, setup, nodes, links);

    window.on_compute_link_at({
        let controller = controller.clone();
        let links = links.clone();
        move |x, y| {
            let links = (0..links.row_count()).filter_map(|index| {
                let link = links.row_data(index)?;
                Some((link.id, link.start_pin_id, link.end_pin_id))
            });
            controller.cache().borrow().find_bezier_link_at_world(
                x,
                y,
                links,
                controller.screen_distance_to_world(8.0),
                50.0,
                20,
            )
        }
    });

    let next_node_id = Rc::new(Cell::new(count + 1));
    window.on_add_node_requested({
        let nodes = nodes.clone();
        move || {
            let id = next_node_id.get();
            next_node_id.set(id + 1);
            nodes.push(NodeData {
                id,
                title: format!("Node {id}").into(),
                category: "Custom".into(),
                status: "Not run".into(),
                x: 120.0,
                y: 120.0,
                selected: false,
            });
        }
    });

    let next_link_id = Rc::new(Cell::new(count + 1));
    window.on_link_requested({
        let window = window.as_weak();
        let controller = controller.clone();
        let links = links.clone();
        move |pin_a, pin_b| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let cache = controller.cache();
            let cache = cache.borrow();
            let (Some(a), Some(b)) = (
                cache.pin_positions.get(&pin_a),
                cache.pin_positions.get(&pin_b),
            ) else {
                return;
            };
            if a.node_id == b.node_id {
                return;
            }
            let types = PinTypes::get(&window);
            let (output, input) = match (a.pin_type, b.pin_type) {
                (a, b) if a == types.get_output() && b == types.get_input() => (pin_a, pin_b),
                (a, b) if a == types.get_input() && b == types.get_output() => (pin_b, pin_a),
                _ => return,
            };
            if links
                .iter()
                .any(|l| l.start_pin_id == output && l.end_pin_id == input)
            {
                return;
            }
            links.push(LinkData {
                id: next_link_id.get(),
                start_pin_id: output,
                end_pin_id: input,
                color: Color::from_rgb_u8(110, 168, 254),
                selected: false,
                line_width: 2.0,
                status: -1,
                ..Default::default()
            });
            next_link_id.set(next_link_id.get() + 1);
        }
    });

    window.on_delete_selected_requested({
        let window = window.as_weak();
        let controller = controller.clone();
        let nodes = nodes.clone();
        let links = links.clone();
        move || {
            let removed: HashSet<i32> = nodes.iter().filter(|n| n.selected).map(|n| n.id).collect();
            if removed.is_empty() {
                return;
            }
            let cache = controller.cache();
            let cache = cache.borrow();
            remove_where(&links, |l| {
                [l.start_pin_id, l.end_pin_id].iter().any(|pin| {
                    cache
                        .pin_positions
                        .get(pin)
                        .is_some_and(|p| removed.contains(&p.node_id))
                })
            });
            drop(cache);
            remove_where(&nodes, |n| removed.contains(&n.id));
            if let Some(window) = window.upgrade() {
                let lifecycle = window.global::<NodeEditorInternalCallbacks>();
                for id in &removed {
                    lifecycle.invoke_remove_node(*id);
                }
            }
        }
    });

    // Worst case for redraw cost: every node moves every frame.
    let anim = Timer::default();
    if env("SPIKE_ANIM", 0) == 1 {
        let nodes = nodes.clone();
        let flip = Cell::new(1.0f32);
        // SPIKE_ANIM_COUNT limits how many nodes move (1 = an ordinary drag).
        let moving: usize = env("SPIKE_ANIM_COUNT", usize::MAX);
        anim.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let d = flip.get();
            flip.set(-d);
            for i in 0..nodes.row_count().min(moving) {
                if let Some(mut n) = nodes.row_data(i) {
                    n.x += d * 3.0;
                    nodes.set_row_data(i, n);
                }
            }
        });
    }

    if let Ok(path) = std::env::var("SPIKE_SNAPSHOT") {
        let weak = window.as_weak();
        Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(w) = weak.upgrade() {
                match w.window().take_snapshot() {
                    Ok(buf) => {
                        let img = image::RgbaImage::from_raw(
                            buf.width(),
                            buf.height(),
                            buf.as_bytes().to_vec(),
                        )
                        .expect("snapshot size");
                        img.save(&path).expect("save snapshot");
                        eprintln!("snapshot {}x{} -> {path}", buf.width(), buf.height());
                    }
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
            let _ = slint::quit_event_loop();
        });
    }

    let secs: u64 = env("SPIKE_EXIT_AFTER", 0);
    if secs > 0 {
        Timer::single_shot(Duration::from_secs(secs), || {
            let _ = slint::quit_event_loop();
        });
    }

    window.run()
}
