//! Workflows (plan WF, ADR-028): the canvas where hosts, devices and the steps that set them up
//! are drawn as connected nodes. This module owns the open graph, turns it into screen rows
//! (the software renderer cannot scale, so Rust positions everything), and saves every change
//! shortly after it is made. Runs arrive with the run engine (WF-SPIKE-02); until then Run says
//! why it is unavailable.

mod canvas;
mod edit;
mod geometry;
mod inspector;
mod run;
mod store;
mod view;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use senvakara_app::{Secret, Workflow, WorkflowProblem};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::{App, WfEdge, WfField, WfLink, WfModule, WfNode, WfProblem, WfRun, Workflows};

use geometry::View;
use view::Machines;

/// The workflow open on the canvas, as edited.
struct Open {
    id: String,
    name: String,
    graph: Workflow,
}

#[derive(Default)]
pub struct State {
    /// Saved workflows as (id, name), in picker order.
    list: RefCell<Vec<(String, String)>>,
    open: RefCell<Option<Open>>,
    view: Cell<View>,
    /// Canvas size in pixels, for the grid.
    size: Cell<(f32, f32)>,
    /// Selected node id, "" for none.
    selected: RefCell<String>,
    /// Which script of the selected node the inspector shows.
    script: Cell<usize>,
    query: RefCell<String>,
    problems: RefCell<Vec<WorkflowProblem>>,
    machines: RefCell<Machines>,
    /// The view when the empty canvas was pressed: panning moves from there.
    pan_from: Cell<Option<View>>,
    /// Saves the open workflow a moment after the last change.
    save_timer: slint::Timer,
    /// Sudo password for the next run; memory only, handed to that run.
    sudo: RefCell<Option<Secret>>,
    nodes: Rc<VecModel<WfNode>>,
    edges: Rc<VecModel<WfEdge>>,
    modules: Rc<VecModel<WfModule>>,
    names: Rc<VecModel<SharedString>>,
    fields: Rc<VecModel<WfField>>,
    links: Rc<VecModel<WfLink>>,
    sel_problems: Rc<VecModel<WfProblem>>,
    /// Node ids behind the "Connect to" picker, after its "choose" entry.
    connect_ids: RefCell<Vec<String>>,
    all_problems: Rc<VecModel<WfProblem>>,
    runs: Rc<VecModel<WfRun>>,
    /// Each node's step in the newest run: (status, changed).
    last_run: RefCell<std::collections::HashMap<String, (String, Option<bool>)>>,
}

/// Replaces a model's rows but keeps the row instances when the count is unchanged, so the
/// card under the pointer is never rebuilt mid-drag (that would drop the press).
fn sync_rows<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

impl App {
    pub fn init_workflows(self: &Rc<Self>, ui: &crate::MainWindow) {
        let w = ui.global::<Workflows>();
        let s = &self.wf;
        w.set_nodes(s.nodes.clone().into());
        w.set_edges(s.edges.clone().into());
        w.set_modules(s.modules.clone().into());
        w.set_names(s.names.clone().into());
        w.set_fields(s.fields.clone().into());
        w.set_links(s.links.clone().into());
        w.set_sel_problems(s.sel_problems.clone().into());
        w.set_problems(s.all_problems.clone().into());
        w.set_runs(s.runs.clone().into());
        // The catalog is fixed while the app runs: the palette is built once, then filtered.
        s.modules
            .set_vec(view::palette_rows(senvakara_app::catalog(), ""));

        macro_rules! wire {
            ($on:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
                let $app = self.clone();
                w.$on(move |$($arg),*| $body);
            }};
        }
        wire!(on_pick, |app, index| app.workflows_pick(index));
        wire!(on_create, |app| app.workflows_create());
        wire!(on_rename, |app, name| app.workflows_rename(&name));
        wire!(on_remove, |app| app.workflows_remove_ask());
        wire!(on_search, |app, text| app.workflows_search(&text));
        wire!(on_add_module, |app, id| app.workflows_add(&id));
        wire!(on_canvas_resized, |app, wd, ht| app
            .workflows_resized(wd, ht));
        wire!(on_canvas_pressed, |app| app.workflows_canvas_pressed());
        wire!(on_canvas_panned, |app, dx, dy| app.workflows_panned(dx, dy));
        wire!(on_canvas_wheel, |app, dx, dy, x, y, zoom| app
            .workflows_wheel(dx, dy, x, y, zoom));
        wire!(on_zoom_step, |app, by| app.workflows_zoom_step(by));
        wire!(on_zoom_reset, |app| app.workflows_zoom_reset());
        wire!(on_node_pressed, |app, id| app.workflows_select(&id));
        wire!(on_node_dragged, |app, id, dx, dy| app
            .workflows_drag(&id, dx, dy));
        wire!(on_node_released, |app, id| app.workflows_dropped(&id));
        wire!(on_link_dragged, |app, from, x, y| app
            .workflows_link_drag(&from, x, y));
        wire!(on_link_released, |app, from, x, y| app
            .workflows_link_drop(&from, x, y));
        wire!(on_key_move, |app, dx, dy| app.workflows_key_move(dx, dy));
        wire!(on_delete_selected, |app| app.workflows_delete_selected());
        wire!(on_select_node, |app, id| app.workflows_select(&id));
        wire!(on_set_title, |app, text| app.workflows_set_title(&text));
        wire!(on_pick_machine, |app, index| app
            .workflows_pick_machine(index));
        wire!(on_field_text, |app, name, text| app
            .workflows_field_text(&name, &text));
        wire!(on_field_checked, |app, name, on| app
            .workflows_field_checked(&name, on));
        wire!(on_field_choice, |app, name, index| app
            .workflows_field_choice(&name, index));
        wire!(on_field_browse, |app, name| app
            .workflows_field_browse(&name));
        wire!(on_pick_script, |app, index| app
            .workflows_pick_script(index));
        wire!(on_script_edited, |app, text| app
            .workflows_script_edited(&text));
        wire!(on_script_reset, |app| app.workflows_script_reset());
        wire!(on_connect_to, |app, index| app.workflows_connect_to(index));
        wire!(on_link_remove, |app, from, to| app
            .workflows_link_remove(&from, &to));
        wire!(on_link_service, |app, from, to, on| app
            .workflows_link_service(&from, &to, on));
        wire!(on_set_permission, |app, which, on| app
            .workflows_set_permission(&which, on));
        wire!(on_run, |app| app.workflows_run_ask());
        wire!(on_sudo_edited, |app, text| app.workflows_sudo(&text));
    }

    /// The Workflows section was opened: read the saved workflows and the machines.
    pub fn workflows_opened(self: &Rc<Self>) {
        self.workflows_reload(None);
    }

    /// Runs `f` with the Workflows global, when the window is still there.
    fn with_view(&self, f: impl FnOnce(&Workflows<'_>)) {
        if let Some(ui) = self.ui.upgrade() {
            f(&ui.global::<Workflows>());
        }
    }
}
