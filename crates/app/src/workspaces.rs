//! Saved workspace commands (WORK-01/02). Targets are stored as references;
//! the UI re-validates each panel when a workspace is opened.

use crate::logging;
use crate::state::{db_err, CmdResult};
use crate::App;
use senvakara_storage::{WorkspaceRecord, WorkspaceSave};

pub(crate) fn workspaces_list(app: &App) -> CmdResult<Vec<WorkspaceRecord>> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .list_workspaces()
        .map_err(|e| db_err("list workspaces", e))
}

pub(crate) fn workspace_save(app: &App, input: WorkspaceSave) -> CmdResult<WorkspaceRecord> {
    let state = &app.state;
    let w = state
        .db
        .lock()
        .unwrap()
        .save_workspace(&input)
        .map_err(|e| db_err("save workspace", e))?;
    logging::info(&format!(
        "workspace saved {} ({} panels)",
        w.id,
        w.panels.len()
    ));
    Ok(w)
}

pub(crate) fn workspace_delete(app: &App, id: String) -> CmdResult<()> {
    let state = &app.state;
    state
        .db
        .lock()
        .unwrap()
        .delete_workspace(&id)
        .map_err(|e| db_err("delete workspace", e))
}
