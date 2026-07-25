use std::sync::Arc;

use tauri::AppHandle;

use crate::error::AppError;

use super::state::AppState;

#[tauri::command]
pub fn export_projects_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    project_ids: Vec<String>,
    dest_path: String,
) -> Result<(), AppError> {
    crate::data::transfer::export_projects(&app, &state.pool, &project_ids, &dest_path)
}

#[tauri::command]
pub fn export_session_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    dest_path: String,
) -> Result<(), AppError> {
    crate::data::transfer::export_session(&app, &state.pool, &session_id, &dest_path)
}

#[tauri::command]
pub fn import_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    archive_path: String,
) -> Result<crate::data::transfer::ImportResult, AppError> {
    crate::data::transfer::import_archive(&app, &state.pool, &archive_path)
}
