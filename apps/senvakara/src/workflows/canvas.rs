//! Canvas handlers: what the user does with the pointer and keys on the canvas, and turning the
//! open graph into rows. A drag only rewrites the moved card and its own curves (ADR-028: about
//! a quarter of a core for 50 nodes); everything else rebuilds the rows, which is cheap.

use std::rc::Rc;

use senvakara_app::{workflow_problems, Workflow};
use slint::{Model, SharedString};

use super::edit;
use super::geometry::{curve, grid, node_rect, out_port, step_zoom, NODE_H, NODE_W};
use super::view::{
    edge_row, last_run_state, node_row, palette_rows, problems_by_node, sentence, title_of,
    zoom_text,
};
use crate::{App, WfEdge, WfProblem};

/// How far an arrow key moves the selected card, in world units.
const KEY_STEP: f32 = 16.0;

impl App {
    /// Rebuilds everything the section shows from the open graph. `inspector` also refills the
    /// inspector's fields; edits typed into the inspector pass `false` so the field being
    /// typed in keeps its cursor.
    pub(super) fn workflows_refresh(self: &Rc<Self>, inspector: bool) {
        let catalog = senvakara_app::catalog();
        let open = self.wf.open.borrow();
        let view = self.wf.view.get();
        let selected = self.wf.selected.borrow().clone();
        let (problems, nodes, edges) = match open.as_ref() {
            Some(o) => {
                let problems = workflow_problems(&o.graph);
                let by_node = problems_by_node(&problems);
                let machines = self.wf.machines.borrow();
                let nodes: Vec<_> = o
                    .graph
                    .nodes
                    .iter()
                    .map(|n| {
                        let mut row =
                            node_row(&o.graph, n, catalog, &machines, &by_node, &view, &selected);
                        // A checked SSH host shows its posture next to its state.
                        if let Some(badge) = self.workflows_badge(n) {
                            row.state_text = format!("{} · {badge}", row.state_text).into();
                        }
                        // An action without problems shows how its step ended last time.
                        if !row.anchor && row.tone == "idle" {
                            if let Some((status, changed)) = self.wf.last_run.borrow().get(&n.id) {
                                let (tone, word) = last_run_state(status, *changed);
                                row.tone = tone.into();
                                row.state_text = word.into();
                            }
                        }
                        row
                    })
                    .collect();
                let edges: Vec<_> = (0..o.graph.edges.len())
                    .map(|i| edge_row(&o.graph, i, &view))
                    .collect();
                (problems, nodes, edges)
            }
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        let all: Vec<WfProblem> = match open.as_ref() {
            Some(o) => problems
                .iter()
                .map(|p| {
                    let node = p.node.clone().unwrap_or_default();
                    let text = match o.graph.node(&node) {
                        Some(n) => format!("{}: {}", title_of(n, catalog), sentence(&p.message)),
                        None => sentence(&p.message),
                    };
                    WfProblem {
                        node: node.into(),
                        text: text.into(),
                    }
                })
                .collect(),
            None => Vec::new(),
        };
        let current = open
            .as_ref()
            .and_then(|o| self.wf.list.borrow().iter().position(|(id, _)| *id == o.id))
            .map_or(-1, |i| i as i32);
        let name = open.as_ref().map(|o| o.name.clone()).unwrap_or_default();
        let is_open = open.is_some();
        drop(open);

        let problem_count = problems.len();
        *self.wf.problems.borrow_mut() = problems;
        let run_reason = self.workflows_run_reason(problem_count);
        let needs_sudo =
            problem_count == 0 && self.workflows_plan().is_some_and(|p| p.needs_root());
        super::sync_rows(&self.wf.nodes, nodes);
        super::sync_rows(&self.wf.edges, edges);
        self.wf.all_problems.set_vec(all);
        let empty = self
            .wf
            .open
            .borrow()
            .as_ref()
            .is_some_and(|o| o.graph.nodes.is_empty());
        let (w, h) = self.wf.size.get();
        self.with_view(|v| {
            v.set_empty(empty);
            v.set_open(is_open);
            v.set_current(current);
            v.set_name(name.into());
            v.set_zoom(view.zoom);
            v.set_zoom_text(zoom_text(view.zoom).into());
            v.set_grid(grid(&view, w, h).into());
            v.set_preview(WfEdge::default());
            v.set_run_reason(run_reason.into());
            v.set_needs_sudo(needs_sudo);
            v.set_selected(selected.into());
        });
        self.workflows_fill_inspector(inspector);
    }

    /// Pan or zoom changed: new positions for every card and curve, nothing else.
    fn workflows_relayout(self: &Rc<Self>) {
        let open = self.wf.open.borrow();
        let Some(o) = open.as_ref() else { return };
        let view = self.wf.view.get();
        for (i, n) in o.graph.nodes.iter().enumerate() {
            if let Some(mut row) = self.wf.nodes.row_data(i) {
                let r = node_rect(&view, n.x, n.y);
                (row.x, row.y, row.w, row.h) = (r.x, r.y, r.w, r.h);
                self.wf.nodes.set_row_data(i, row);
            }
        }
        for i in 0..o.graph.edges.len() {
            self.wf.edges.set_row_data(i, edge_row(&o.graph, i, &view));
        }
        drop(open);
        let (w, h) = self.wf.size.get();
        self.with_view(|v| {
            v.set_zoom(view.zoom);
            v.set_zoom_text(zoom_text(view.zoom).into());
            v.set_grid(grid(&view, w, h).into());
        });
    }

    pub(super) fn workflows_resized(self: &Rc<Self>, w: f32, h: f32) {
        self.wf.size.set((w, h));
        let view = self.wf.view.get();
        self.with_view(|v| v.set_grid(grid(&view, w, h).into()));
    }

    pub(super) fn workflows_search(self: &Rc<Self>, text: &str) {
        *self.wf.query.borrow_mut() = text.to_string();
        self.wf
            .modules
            .set_vec(palette_rows(senvakara_app::catalog(), text));
        self.with_view(|v| v.set_query(text.into()));
    }

    /// Adds a module in the middle of the visible canvas, a little offset per node so new cards
    /// do not cover each other, and selects it.
    pub(super) fn workflows_add(self: &Rc<Self>, module: &str) {
        let view = self.wf.view.get();
        let (w, h) = self.wf.size.get();
        let (cx, cy) = view.to_world(w / 2.0, h / 2.0);
        let count = self
            .wf
            .open
            .borrow()
            .as_ref()
            .map_or(0, |o| o.graph.nodes.len());
        let shift = (count % 6) as f32 * 24.0;
        let mut id = String::new();
        self.workflows_edit(|wf| {
            id = edit::add_node(
                wf,
                module,
                cx - NODE_W / 2.0 + shift,
                cy - NODE_H / 2.0 + shift,
            );
        });
        if !id.is_empty() {
            self.workflows_select(&id);
        }
    }

    pub(super) fn workflows_select(self: &Rc<Self>, id: &str) {
        if *self.wf.selected.borrow() != id {
            *self.wf.selected.borrow_mut() = id.to_string();
            self.wf.script.set(0);
        }
        self.workflows_error("");
        self.workflows_refresh(true);
    }

    pub(super) fn workflows_canvas_pressed(self: &Rc<Self>) {
        self.wf.pan_from.set(Some(self.wf.view.get()));
        if !self.wf.selected.borrow().is_empty() {
            self.workflows_select("");
        }
    }

    pub(super) fn workflows_panned(self: &Rc<Self>, dx: f32, dy: f32) {
        let Some(from) = self.wf.pan_from.get() else {
            return;
        };
        let mut view = from;
        view.pan_x += dx;
        view.pan_y += dy;
        self.wf.view.set(view);
        self.workflows_relayout();
        self.workflows_schedule_save();
    }

    pub(super) fn workflows_wheel(self: &Rc<Self>, dx: f32, dy: f32, x: f32, y: f32, zoom: bool) {
        let mut view = self.wf.view.get();
        if zoom {
            if dy == 0.0 {
                return;
            }
            view.zoom_at(step_zoom(view.zoom, if dy > 0.0 { 1 } else { -1 }), x, y);
        } else {
            view.pan_x += dx;
            view.pan_y += dy;
        }
        self.wf.view.set(view);
        self.workflows_relayout();
        self.workflows_schedule_save();
    }

    pub(super) fn workflows_zoom_step(self: &Rc<Self>, by: i32) {
        let mut view = self.wf.view.get();
        let (w, h) = self.wf.size.get();
        view.zoom_at(step_zoom(view.zoom, by), w / 2.0, h / 2.0);
        self.wf.view.set(view);
        self.workflows_relayout();
        self.workflows_schedule_save();
    }

    pub(super) fn workflows_zoom_reset(self: &Rc<Self>) {
        let mut view = self.wf.view.get();
        let (w, h) = self.wf.size.get();
        view.zoom_at(1.0, w / 2.0, h / 2.0);
        self.wf.view.set(view);
        self.workflows_relayout();
        self.workflows_schedule_save();
    }

    /// A card moved by (`dx`, `dy`) screen pixels: rewrite its row and its own curves only.
    pub(super) fn workflows_drag(self: &Rc<Self>, id: &str, dx: f32, dy: f32) {
        let view = self.wf.view.get();
        let mut open = self.wf.open.borrow_mut();
        let Some(o) = open.as_mut() else { return };
        let Some(i) = o.graph.nodes.iter().position(|n| n.id == id) else {
            return;
        };
        o.graph.nodes[i].x += dx / view.zoom;
        o.graph.nodes[i].y += dy / view.zoom;
        let n = &o.graph.nodes[i];
        if let Some(mut row) = self.wf.nodes.row_data(i) {
            let r = node_rect(&view, n.x, n.y);
            (row.x, row.y) = (r.x, r.y);
            self.wf.nodes.set_row_data(i, row);
        }
        for (e, edge) in o.graph.edges.iter().enumerate() {
            if edge.from == id || edge.to == id {
                self.wf.edges.set_row_data(e, edge_row(&o.graph, e, &view));
            }
        }
    }

    /// The drag ended: snap to whole units and save.
    pub(super) fn workflows_dropped(self: &Rc<Self>, id: &str) {
        let pos = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(id).map(|n| (n.x, n.y)));
        if let Some((x, y)) = pos {
            self.workflows_edit(|wf| edit::move_node(wf, id, x, y));
            self.workflows_relayout();
        }
    }

    /// A connection is being dragged from `from` to the pointer.
    pub(super) fn workflows_link_drag(self: &Rc<Self>, from: &str, x: f32, y: f32) {
        let view = self.wf.view.get();
        let start = self.wf.open.borrow().as_ref().and_then(|o| {
            o.graph
                .node(from)
                .map(|n| out_port(&node_rect(&view, n.x, n.y)))
        });
        let Some(start) = start else { return };
        let c = curve(start, (x, y), view.zoom);
        self.with_view(|v| {
            v.set_preview(WfEdge {
                x: c.x,
                y: c.y,
                w: c.w,
                h: c.h,
                commands: c.commands.into(),
                kind: SharedString::from("after"),
            })
        });
    }

    /// The connection was dropped at (`x`, `y`): connect to the card there, if any.
    pub(super) fn workflows_link_drop(self: &Rc<Self>, from: &str, x: f32, y: f32) {
        self.with_view(|v| v.set_preview(WfEdge::default()));
        let view = self.wf.view.get();
        let target = self.wf.open.borrow().as_ref().and_then(|o| {
            o.graph
                .nodes
                .iter()
                .rev()
                .find(|n| node_rect(&view, n.x, n.y).contains(x, y))
                .map(|n| n.id.clone())
        });
        let Some(to) = target.filter(|t| t != from) else {
            return;
        };
        let mut result = Ok(());
        self.workflows_edit(|wf| {
            result = edit::connect(wf, senvakara_app::catalog(), from, &to).map(|_| ());
        });
        match result {
            Ok(()) => {
                self.workflows_error("");
                self.workflows_refresh(true);
            }
            Err(text) => self.workflows_error(&text),
        }
    }

    pub(super) fn workflows_key_move(self: &Rc<Self>, dx: i32, dy: i32) {
        let id = self.wf.selected.borrow().clone();
        if id.is_empty() {
            return;
        }
        let pos = self
            .wf
            .open
            .borrow()
            .as_ref()
            .and_then(|o| o.graph.node(&id).map(|n| (n.x, n.y)));
        if let Some((x, y)) = pos {
            self.workflows_edit(|wf: &mut Workflow| {
                edit::move_node(wf, &id, x + dx as f32 * KEY_STEP, y + dy as f32 * KEY_STEP)
            });
            self.workflows_relayout();
        }
    }

    pub(super) fn workflows_delete_selected(self: &Rc<Self>) {
        let id = self.wf.selected.borrow().clone();
        if id.is_empty() {
            return;
        }
        self.workflows_edit(|wf| edit::delete_node(wf, &id));
        self.workflows_select("");
    }
}
