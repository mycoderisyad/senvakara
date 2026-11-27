//! Saved workflows of the Workflows canvas (plan WF, ADR-028), which
//! replaces Automations. Graphs are stored as JSON and checked against the
//! bundled catalog of `senvakara-workflow`; recipes saved before the canvas
//! existed are converted once at start.

use crate::logging;
use crate::state::{db_err, CmdResult};
use crate::App;
use senvakara_core::{CoreError, ErrorCode};
use senvakara_storage::{Db, WorkflowRecord, WorkflowSave};
use senvakara_workflow::{from_recipe, validate, Catalog, Problem, RecipeStepIn, Workflow};
use std::sync::OnceLock;

/// A saved workflow with its graph parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowView {
    pub id: String,
    pub name: String,
    pub graph: Workflow,
    /// Set when the workflow came from an Automations recipe.
    pub from_recipe_id: Option<String>,
    pub updated_at: String,
}

/// The bundled module catalog, parsed on first use.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    // Embedded at build time; `the_bundled_catalog_loads` keeps it valid.
    CATALOG.get_or_init(|| Catalog::bundled().expect("bundled workflow catalog"))
}

fn view(record: WorkflowRecord) -> CmdResult<WorkflowView> {
    let graph: Workflow = serde_json::from_value(record.graph).map_err(|e| {
        CoreError::new(
            ErrorCode::InvalidInput,
            format!("workflow {} could not be read: {e}", record.name),
        )
    })?;
    Ok(WorkflowView {
        id: record.id,
        name: record.name,
        graph,
        from_recipe_id: record.from_recipe_id,
        updated_at: record.updated_at,
    })
}

/// Every saved workflow. One that cannot be read is skipped and logged, so
/// it does not hide the others.
pub(crate) fn workflows_list(app: &App) -> CmdResult<Vec<WorkflowView>> {
    list(&app.state.db.lock().unwrap())
}

fn list(db: &Db) -> CmdResult<Vec<WorkflowView>> {
    let records = db
        .list_workflows()
        .map_err(|e| db_err("list workflows", e))?;
    Ok(records
        .into_iter()
        .filter_map(|r| {
            view(r)
                .map_err(|e| logging::warn(&format!("workflow skipped: {}", e.message)))
                .ok()
        })
        .collect())
}

/// Saves a workflow as drawn. Unfinished graphs are allowed: the rules are
/// checked before a run and shown on the canvas, not here.
pub(crate) fn workflow_save(
    app: &App,
    id: Option<String>,
    name: String,
    graph: &Workflow,
) -> CmdResult<WorkflowView> {
    save(&app.state.db.lock().unwrap(), id, name, graph)
}

fn save(db: &Db, id: Option<String>, name: String, graph: &Workflow) -> CmdResult<WorkflowView> {
    let graph = serde_json::to_value(graph)
        .map_err(|e| CoreError::internal(format!("encode workflow: {e}")))?;
    let record = db
        .save_workflow(&WorkflowSave { id, name, graph })
        .map_err(|e| db_err("save workflow", e))?;
    view(record)
}

pub(crate) fn workflow_delete(app: &App, id: &str) -> CmdResult<()> {
    app.state
        .db
        .lock()
        .unwrap()
        .delete_workflow(id)
        .map_err(|e| db_err("delete workflow", e))
}

/// What is wrong with a graph, for the canvas; empty when it may run.
pub fn workflow_problems(graph: &Workflow) -> Vec<Problem> {
    validate(graph, catalog()).err().unwrap_or_default()
}

/// Converts every recipe that has no workflow yet. Recipes are kept, so
/// nothing is lost if a conversion fails; a failure is logged and the
/// recipe is tried again at the next start.
pub(crate) fn convert_recipes(app: &App) {
    convert(&app.state.db.lock().unwrap());
}

fn convert(db: &Db) {
    let recipes = match db.list_recipes() {
        Ok(r) => r,
        Err(e) => {
            logging::warn(&format!("recipes not converted: {e}"));
            return;
        }
    };
    let mut converted = 0;
    for recipe in &recipes {
        let steps: Vec<RecipeStepIn<'_>> = recipe
            .steps
            .iter()
            .map(|s| RecipeStepIn {
                kind: &s.kind,
                params: &s.params,
            })
            .collect();
        let result = from_recipe(
            recipe.target_route_id.as_deref(),
            &steps,
            recipe.permissions.install,
            recipe.permissions.shell,
        )
        .and_then(|wf| serde_json::to_value(&wf).map_err(|e| e.to_string()))
        .and_then(|graph| {
            db.import_recipe_workflow(&recipe.id, &recipe.name, &graph)
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(Some(_)) => converted += 1,
            Ok(None) => {}
            Err(e) => logging::warn(&format!("recipe {} not converted: {e}", recipe.id)),
        }
    }
    if converted > 0 {
        logging::info(&format!("converted {converted} recipes to workflows"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use senvakara_storage::{RecipePermissions, RecipeSave, RecipeStep};
    use senvakara_workflow::{Binding, Node};
    use serde_json::json;

    #[test]
    fn saved_recipes_become_workflows_once_and_stay_runnable() {
        let mut db = Db::open_in_memory().unwrap();
        db.save_recipe(&RecipeSave {
            id: None,
            name: "Smoke test".into(),
            target_route_id: Some("route-1".into()),
            steps: vec![
                RecipeStep {
                    kind: "launch_app".into(),
                    params: json!({"package": "dev.x.app"}),
                },
                RecipeStep {
                    kind: "wait".into(),
                    params: json!({"seconds": 2}),
                },
            ],
            permissions: RecipePermissions::default(),
        })
        .unwrap();
        convert(&db);
        convert(&db);
        let all = list(&db).unwrap();
        assert_eq!(all.len(), 1);
        let wf = &all[0];
        assert_eq!(wf.name, "Smoke test");
        assert!(wf.from_recipe_id.is_some());
        assert_eq!(
            wf.graph.node("device").unwrap().binding,
            Some(Binding::Device {
                route_id: "route-1".into()
            })
        );
        assert!(workflow_problems(&wf.graph).is_empty());
        // The recipe itself is kept.
        assert_eq!(db.list_recipes().unwrap().len(), 1);
    }

    #[test]
    fn drafts_save_with_problems_and_the_canvas_sees_them() {
        let db = Db::open_in_memory().unwrap();
        let mut graph = Workflow::default();
        graph.nodes.push(Node {
            id: "s".into(),
            module: "automation.custom_script".into(),
            title: String::new(),
            x: 10.0,
            y: 20.0,
            params: Default::default(),
            binding: None,
            scripts: Default::default(),
        });
        let saved = save(&db, None, "Draft".into(), &graph).unwrap();
        assert_eq!(saved.graph, graph);
        let problems = workflow_problems(&saved.graph);
        assert!(!problems.is_empty());
        assert!(problems.iter().all(|p| p.node.as_deref() == Some("s")));
    }

    #[test]
    fn an_unreadable_graph_is_skipped_not_fatal() {
        let db = Db::open_in_memory().unwrap();
        db.save_workflow(&WorkflowSave {
            id: None,
            name: "Broken".into(),
            graph: json!({"nodes": "not a list"}),
        })
        .unwrap();
        save(&db, None, "Fine".into(), &Workflow::default()).unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["Fine"]);
    }
}
