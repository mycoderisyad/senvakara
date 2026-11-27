//! Visual Workflow Builder model (plan WF, ADR-028/029): the graph a user
//! draws on the Workflows canvas, the bundled module catalog, parameter
//! checks with safe script preambles, and run plans.
//!
//! Pure: no I/O, no SSH, no database. `crates/app` runs plans; storage keeps
//! the graph as JSON.

mod catalog;
mod graph;
mod legacy;
mod module;
mod params;
mod plan;
mod posture;
mod session;
mod template;
mod validate;

pub use catalog::Catalog;
pub use graph::{
    Binding, Edge, EdgeKind, Node, Permissions, Viewport, Workflow, GRAPH_VERSION, MAX_EDGES,
    MAX_NODES,
};
pub use legacy::{from_recipe, RecipeStepIn};
pub use module::{
    parse_os_release, BindKind, Category, Executor, Guard, Manifest, Module, ParamKind, ParamSpec,
    Permission, GUARD_SECS, MAX_SCRIPT_LEN, SCRIPT_FILES,
};
pub use params::{check_params, preamble, shell_quote, valid_package, ParamValue};
pub use plan::{plan, Plan};
pub use posture::{control_info, Control, ControlInfo, Posture, Status, CONTROLS, POSTURE_SCRIPT};
pub use session::{session_exports, Session};
pub use template::{add_vps_baseline, VPS_BASELINE};
pub use validate::{validate, CheckedNode, Problem};
