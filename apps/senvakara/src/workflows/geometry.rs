//! Canvas geometry: world units (as stored in the graph) to screen pixels, zoom steps, the
//! curve of a connection and what lies under the pointer. The software renderer cannot scale
//! elements (ADR-028), so every screen position and size is computed here.

/// Size of a node card at zoom 1, in world units.
pub const NODE_W: f32 = 220.0;
pub const NODE_H: f32 = 84.0;
/// Zoom levels the buttons and Ctrl+wheel step through.
pub const ZOOM_STEPS: [f32; 7] = [0.5, 0.67, 0.8, 1.0, 1.25, 1.5, 2.0];
/// Extra room around a curve's bounding box for the stroke.
const STROKE_PAD: f32 = 3.0;

/// Pan (screen pixels) and zoom of the canvas: `screen = pan + world * zoom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub pan_x: f32,
    pub pan_y: f32,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan_x: 40.0,
            pan_y: 40.0,
            zoom: 1.0,
        }
    }
}

impl View {
    pub fn to_screen(self, x: f32, y: f32) -> (f32, f32) {
        (self.pan_x + x * self.zoom, self.pan_y + y * self.zoom)
    }

    pub fn to_world(self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.pan_x) / self.zoom, (y - self.pan_y) / self.zoom)
    }

    /// Changes the zoom while the world point under (`x`, `y`) stays under it.
    pub fn zoom_at(&mut self, zoom: f32, x: f32, y: f32) {
        let zoom = zoom.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        let (wx, wy) = self.to_world(x, y);
        self.zoom = zoom;
        self.pan_x = x - wx * zoom;
        self.pan_y = y - wy * zoom;
    }
}

/// The next zoom step in or out from `zoom` (`dir` > 0 zooms in).
pub fn step_zoom(zoom: f32, dir: i32) -> f32 {
    if dir > 0 {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|z| *z > zoom + 0.001)
            .unwrap_or(ZOOM_STEPS[ZOOM_STEPS.len() - 1])
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|z| *z < zoom - 0.001)
            .unwrap_or(ZOOM_STEPS[0])
    }
}

/// A screen rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// The screen rectangle of a node at world position (`x`, `y`).
pub fn node_rect(view: &View, x: f32, y: f32) -> Rect {
    let (sx, sy) = view.to_screen(x, y);
    Rect {
        x: sx,
        y: sy,
        w: NODE_W * view.zoom,
        h: NODE_H * view.zoom,
    }
}

/// Where connections leave a node (right edge) and arrive (left edge), in screen pixels.
pub fn out_port(r: &Rect) -> (f32, f32) {
    (r.x + r.w, r.y + r.h / 2.0)
}

pub fn in_port(r: &Rect) -> (f32, f32) {
    (r.x, r.y + r.h / 2.0)
}

/// A curve drawn by one `Path` sized to its own bounding box, so partial rendering can skip
/// curves away from what changed. `commands` are relative to (`x`, `y`).
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub commands: String,
}

/// A horizontal S-curve from `a` to `b`. When `b` is left of `a` the curve still leaves to the
/// right and arrives from the left, so the direction stays readable.
pub fn curve(a: (f32, f32), b: (f32, f32), zoom: f32) -> Curve {
    let bend = ((b.0 - a.0).abs() / 2.0).max(48.0 * zoom);
    let (c1x, c2x) = (a.0 + bend, b.0 - bend);
    let min_x = a.0.min(b.0).min(c1x).min(c2x) - STROKE_PAD;
    let max_x = a.0.max(b.0).max(c1x).max(c2x) + STROKE_PAD;
    let min_y = a.1.min(b.1) - STROKE_PAD;
    let max_y = a.1.max(b.1) + STROKE_PAD;
    let (ox, oy) = (min_x, min_y);
    Curve {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
        commands: format!(
            "M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
            a.0 - ox,
            a.1 - oy,
            c1x - ox,
            a.1 - oy,
            c2x - ox,
            b.1 - oy,
            b.0 - ox,
            b.1 - oy
        ),
    }
}

/// Grid lines every 96 world units across a canvas of `w`×`h` pixels, as path commands.
pub fn grid(view: &View, w: f32, h: f32) -> String {
    let step = 96.0 * view.zoom;
    let mut out = String::new();
    if step < 8.0 || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let mut x = view.pan_x.rem_euclid(step);
    while x < w {
        out.push_str(&format!("M {x:.1} 0 L {x:.1} {h:.1} "));
        x += step;
    }
    let mut y = view.pan_y.rem_euclid(step);
    while y < h {
        out.push_str(&format!("M 0 {y:.1} L {w:.1} {y:.1} "));
        y += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_and_world_coordinates_round_trip() {
        let view = View {
            pan_x: 30.0,
            pan_y: -10.0,
            zoom: 1.5,
        };
        let (sx, sy) = view.to_screen(100.0, 40.0);
        assert_eq!((sx, sy), (180.0, 50.0));
        assert_eq!(view.to_world(sx, sy), (100.0, 40.0));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut view = View::default();
        let before = view.to_world(400.0, 300.0);
        view.zoom_at(1.5, 400.0, 300.0);
        let after = view.to_world(400.0, 300.0);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
        view.zoom_at(9.0, 0.0, 0.0);
        assert_eq!(view.zoom, 2.0);
    }

    #[test]
    fn zoom_steps_stop_at_both_ends() {
        assert_eq!(step_zoom(1.0, 1), 1.25);
        assert_eq!(step_zoom(1.0, -1), 0.8);
        assert_eq!(step_zoom(2.0, 1), 2.0);
        assert_eq!(step_zoom(0.5, -1), 0.5);
        // A zoom between steps goes to the neighbouring step.
        assert_eq!(step_zoom(0.9, 1), 1.0);
    }

    #[test]
    fn nodes_scale_with_the_zoom_and_are_hit_where_drawn() {
        let view = View {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 0.5,
        };
        let r = node_rect(&view, 200.0, 100.0);
        assert_eq!((r.x, r.y, r.w, r.h), (100.0, 50.0, 110.0, 42.0));
        assert!(r.contains(105.0, 60.0));
        assert!(!r.contains(99.0, 60.0));
        assert_eq!(out_port(&r), (210.0, 71.0));
        assert_eq!(in_port(&r), (100.0, 71.0));
    }

    #[test]
    fn a_curve_sits_in_its_own_box() {
        let c = curve((100.0, 50.0), (300.0, 150.0), 1.0);
        assert_eq!((c.x, c.y), (97.0, 47.0));
        assert_eq!((c.w, c.h), (206.0, 106.0));
        assert!(c.commands.starts_with("M 3.0 3.0 C 103.0 3.0"));
        // Backwards: the box widens to hold the bends.
        let back = curve((300.0, 50.0), (100.0, 50.0), 1.0);
        assert!(back.x < 100.0 && back.x + back.w > 300.0);
    }

    #[test]
    fn the_grid_follows_pan_and_zoom() {
        let view = View {
            pan_x: 10.0,
            pan_y: 0.0,
            zoom: 1.0,
        };
        let g = grid(&view, 200.0, 100.0);
        assert!(g.starts_with("M 10.0 0 L 10.0 100.0 M 106.0 0"));
        let tiny = View { zoom: 0.05, ..view };
        assert!(grid(&tiny, 200.0, 100.0).is_empty());
    }
}
