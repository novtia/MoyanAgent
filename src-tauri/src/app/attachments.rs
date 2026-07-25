use std::sync::Arc;

use serde::Deserialize;
use tauri::AppHandle;

use crate::data::{paths, session};
use crate::error::AppError;
use crate::media::images;

use super::messages::reload_message;
use super::state::AppState;

#[tauri::command]
pub async fn quote_message_as_attachments(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    message_id: String,
) -> Result<Vec<images::AttachmentDraft>, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        const MAX_ATTACH: usize = 8;
        let conn = state.conn()?;
        let mut msg = reload_message(&conn, &message_id)?;
        if msg.session_id != session_id {
            return Err(AppError::Invalid("message not in session".into()));
        }
        msg.images.sort_by_key(|i| i.ord);
        let mut out = Vec::new();
        for img in msg.images {
            if img.mime.starts_with("image/")
                && matches!(img.role.as_str(), "input" | "output" | "edited")
            {
                let d = images::clone_image_as_draft(&app, &conn, &session_id, &img.id)?;
                out.push(d);
                if out.len() >= MAX_ATTACH {
                    break;
                }
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

// ????????? Attachments ?????????

#[tauri::command]
pub async fn add_attachment_from_path(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    path: String,
) -> Result<images::AttachmentDraft, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = state.conn()?;
        images::save_path_as_attachment(&app, &conn, &session_id, std::path::Path::new(&path))
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttachBytesArgs {
    pub(crate) session_id: String,
    pub(crate) name: Option<String>,
    pub(crate) bytes: Vec<u8>,
}

#[tauri::command]
pub async fn add_attachment_from_bytes(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    args: AttachBytesArgs,
) -> Result<images::AttachmentDraft, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = state.conn()?;
        images::save_bytes_as_attachment(
            &app,
            &conn,
            &args.session_id,
            args.name.as_deref(),
            &args.bytes,
        )
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttachUrlArgs {
    pub(crate) session_id: String,
    pub(crate) url: String,
}

#[tauri::command]
pub fn add_url_attachment(
    state: tauri::State<Arc<AppState>>,
    args: AttachUrlArgs,
) -> Result<images::AttachmentDraft, AppError> {
    let conn = state.conn()?;
    images::save_url_as_attachment(&conn, &args.session_id, &args.url)
}

#[tauri::command]
pub fn remove_attachment_draft(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    if !img.rel_path.trim().is_empty() {
        if let Ok(abs) = paths::abs_from_rel(&app, &img.rel_path) {
            let _ = std::fs::remove_file(&abs);
        }
    }
    if let Some(thumb) = &img.thumb_rel_path {
        if let Ok(abs) = paths::abs_from_rel(&app, thumb) {
            let _ = std::fs::remove_file(&abs);
        }
    }
    let conn = state.conn()?;
    conn.execute(
        "DELETE FROM message_images WHERE id=?1 AND message_id IS NULL",
        rusqlite::params![image_id],
    )?;
    Ok(())
}
