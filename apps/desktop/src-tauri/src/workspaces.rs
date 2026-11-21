//! Saved workspace commands (WORK-01/02). Targets are stored as references;
//! the UI re-validates each panel when a workspace is opened.

use crate::logging;
use crate::state::{db_err, AppState, CmdResult};
use senvakara_storage::{WorkspaceRecord, WorkspaceSave};
use tauri::State;

#[tauri::command]
pub fn workspaces_list(state: State<'_, AppState>) -> CmdResult<Vec<WorkspaceRecord>> {
    state
        .db
        .lock()
        .unwrap()
        .list_workspaces()
        .map_err(|e| db_err("list workspaces", e))
}

#[tauri::command]
pub fn workspace_save(
    state: State<'_, AppState>,
    input: WorkspaceSave,
) -> CmdResult<WorkspaceRecord> {
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

#[tauri::command]
pub fn workspace_delete(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .db
        .lock()
        .unwrap()
        .delete_workspace(&id)
        .map_err(|e| db_err("delete workspace", e))
}
