//! Fallback canvas spike. Rust keeps the graph in world units and pushes
//! screen-space rows to Slint; a node move updates that node's row and only
//! the rows of its own edges.
//!
//! Env knobs: `SPIKE_NODES` (50), `SPIKE_ZOOM` (1.0), `SPIKE_ANIM=1` (move one
//! node every 16 ms), `SPIKE_GRID=0` (no grid), `SPIKE_SNAPSHOT=<png>`.

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{cell::RefCell, rc::Rc, time::Duration};

slint::include_modules!();

const NODE_W: f32 = 200.0;
const NODE_H: f32 = 72.0;
const PAN: (f32, f32) = (40.0, 40.0);
const CATEGORIES: [&str; 6] = ["Host", "Security", "Database", "App", "Deployment", "Monitoring"];

fn env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct WNode {
    id: i32,
    title: SharedString,
    category: SharedString,
    x: f32,
    y: f32,
}

struct World {
    nodes: Vec<WNode>,
    edges: Vec<(usize, usize)>,
    /// node index -> indexes of edges touching it
    touching: Vec<Vec<usize>>,
    zoom: f32,
    node_rows: Rc<VecModel<CNode>>,
    edge_rows: Rc<VecModel<CEdge>>,
}

impl World {
    fn screen(&self, wx: f32, wy: f32) -> (f32, f32) {
        (PAN.0 + wx * self.zoom, PAN.1 + wy * self.zoom)
    }

    fn node_row(&self, i: usize) -> CNode {
        let n = &self.nodes[i];
        let (x, y) = self.screen(n.x, n.y);
        CNode {
            id: n.id,
            title: n.title.clone(),
            category: n.category.clone(),
            status: "Up to date".into(),
            x,
            y,
            w: NODE_W * self.zoom,
            h: NODE_H * self.zoom,
            selected: false,
        }
    }

    /// Cubic bezier from A's output port to B's input port, in a Path sized
    /// to the curve's bounding box.
    fn edge_row(&self, e: usize) -> CEdge {
        let (a, b) = self.edges[e];
        let (ax, ay) = self.screen(self.nodes[a].x + NODE_W, self.nodes[a].y + NODE_H / 2.0);
        let (bx, by) = self.screen(self.nodes[b].x, self.nodes[b].y + NODE_H / 2.0);
        let bend = ((bx - ax).abs() / 2.0).max(40.0 * self.zoom);
        let (c1x, c2x) = (ax + bend, bx - bend);
        let pad = 3.0;
        let min_x = ax.min(bx).min(c1x).min(c2x) - pad;
        let max_x = ax.max(bx).max(c1x).max(c2x) + pad;
        let min_y = ay.min(by) - pad;
        let max_y = ay.max(by) + pad;
        let (ox, oy) = (min_x, min_y);
        CEdge {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
            commands: format!(
                "M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
                ax - ox,
                ay - oy,
                c1x - ox,
                ay - oy,
                c2x - ox,
                by - oy,
                bx - ox,
                by - oy
            )
            .into(),
        }
    }

    fn rebuild(&self) {
        let nodes: Vec<CNode> = (0..self.nodes.len()).map(|i| self.node_row(i)).collect();
        let edges: Vec<CEdge> = (0..self.edges.len()).map(|e| self.edge_row(e)).collect();
        self.node_rows.set_vec(nodes);
        self.edge_rows.set_vec(edges);
    }

    /// Move one node by a screen-space delta: one node row, its edge rows.
    fn move_by(&mut self, i: usize, dx: f32, dy: f32) {
        self.nodes[i].x += dx / self.zoom;
        self.nodes[i].y += dy / self.zoom;
        self.node_rows.set_row_data(i, self.node_row(i));
        for &e in &self.touching[i] {
            self.edge_rows.set_row_data(e, self.edge_row(e));
        }
    }
}

fn grid(zoom: f32, enabled: bool) -> SharedString {
    if !enabled {
        return SharedString::default();
    }
    let step = 96.0 * zoom;
    let (w, h) = (2560.0f32, 1600.0f32);
    let mut s = String::new();
    let mut x = PAN.0 % step;
    while x < w {
        s.push_str(&format!("M {x:.1} 0 L {x:.1} {h} "));
        x += step;
    }
    let mut y = PAN.1 % step;
    while y < h {
        s.push_str(&format!("M 0 {y:.1} L {w} {y:.1} "));
        y += step;
    }
    s.into()
}

fn main() -> Result<(), slint::PlatformError> {
    let count: usize = env("SPIKE_NODES", 50);
    let grid_on = env("SPIKE_GRID", 1) == 1;
    let window = App::new()?;

    let nodes: Vec<WNode> = (0..count)
        .map(|i| WNode {
            id: i as i32 + 1,
            title: format!("Node {}", i + 1).into(),
            category: CATEGORIES[i % CATEGORIES.len()].into(),
            x: (i % 8) as f32 * 260.0,
            y: (i / 8) as f32 * 120.0,
        })
        .collect();
    let edges: Vec<(usize, usize)> = (0..count.saturating_sub(1)).map(|i| (i, i + 1)).collect();
    let mut touching = vec![Vec::new(); count];
    for (e, &(a, b)) in edges.iter().enumerate() {
        touching[a].push(e);
        touching[b].push(e);
    }
    let world = Rc::new(RefCell::new(World {
        nodes,
        edges,
        touching,
        zoom: env("SPIKE_ZOOM", 1.0),
        node_rows: Rc::new(VecModel::default()),
        edge_rows: Rc::new(VecModel::default()),
    }));
    {
        let w = world.borrow();
        window.set_nodes(ModelRc::from(w.node_rows.clone()));
        window.set_edges(ModelRc::from(w.edge_rows.clone()));
        window.set_zoom(w.zoom);
        window.set_grid(grid(w.zoom, grid_on));
        w.rebuild();
    }

    window.on_zoom_to({
        let world = world.clone();
        let weak = window.as_weak();
        move |z| {
            let mut w = world.borrow_mut();
            w.zoom = z;
            w.rebuild();
            if let Some(win) = weak.upgrade() {
                win.set_zoom(z);
                win.set_grid(grid(z, grid_on));
            }
        }
    });
    window.on_node_dragged({
        let world = world.clone();
        move |id, dx, dy| {
            let mut w = world.borrow_mut();
            if let Some(i) = w.nodes.iter().position(|n| n.id == id) {
                w.move_by(i, dx, dy);
            }
        }
    });
    window.on_node_pressed(|_| {});
    window.on_node_released(|_| {});

    let anim = Timer::default();
    if env("SPIKE_ANIM", 0) == 1 {
        let world = world.clone();
        let flip = std::cell::Cell::new(3.0f32);
        anim.start(TimerMode::Repeated, Duration::from_millis(16), move || {
            let d = flip.get();
            flip.set(-d);
            world.borrow_mut().move_by(0, d, 0.0);
        });
    }

    if let Ok(path) = std::env::var("SPIKE_SNAPSHOT") {
        let weak = window.as_weak();
        Timer::single_shot(Duration::from_millis(1500), move || {
            if let Some(w) = weak.upgrade() {
                match w.window().take_snapshot() {
                    Ok(buf) => {
                        image::RgbaImage::from_raw(buf.width(), buf.height(), buf.as_bytes().to_vec())
                            .expect("snapshot size")
                            .save(&path)
                            .expect("save snapshot");
                        eprintln!("snapshot -> {path}");
                    }
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
            let _ = slint::quit_event_loop();
        });
    }

    let _ = window.get_nodes().row_count();
    window.run()
}
