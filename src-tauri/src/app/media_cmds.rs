use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tauri::AppHandle;

use crate::data::{paths, session};
use crate::error::AppError;
use crate::media::{editor, images};

use super::dto::{decorate_image, ImageRefAbs};
use super::state::AppState;

#[tauri::command]
pub fn get_image_abs_path(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<String, AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    let abs = paths::abs_from_rel(&app, &img.rel_path)?;
    Ok(abs.to_string_lossy().to_string())
}

#[derive(Debug, Deserialize)]
pub(crate) struct EditImageArgs {
    pub(crate) image_id: String,
    pub(crate) op: editor::EditOp,
}

#[tauri::command]
pub fn edit_image(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: EditImageArgs,
) -> Result<ImageRefAbs, AppError> {
    let img = {
        let conn = state.conn()?;
        session::get_image(&conn, &args.image_id)?
    };
    let bytes = images::read_image_bytes(&app, &img)?;
    let result = editor::apply(&bytes, &img.mime, &args.op)?;
    let session_id = {
        let conn = state.conn()?;
        session::image_session_id(&conn, &args.image_id)?
    };
    let conn = state.conn()?;
    let new_ref =
        images::write_edited_image(&app, &conn, &session_id, &result.bytes, &result.mime)?;
    Ok(decorate_image(&app, new_ref))
}

// ????????? Export ?????????

#[derive(Debug, Deserialize)]
pub(crate) struct ExportArgs {
    pub(crate) image_id: String,
    pub(crate) dest_path: String,
}

#[tauri::command]
pub fn export_image(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: ExportArgs,
) -> Result<(), AppError> {
    let img = {
        let conn = state.conn()?;
        session::get_image(&conn, &args.image_id)?
    };
    let abs = paths::abs_from_rel(&app, &img.rel_path)?;
    std::fs::copy(&abs, PathBuf::from(&args.dest_path))?;
    Ok(())
}

#[tauri::command]
pub fn export_media(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: ExportArgs,
) -> Result<(), AppError> {
    export_image(state, app, args)
}
