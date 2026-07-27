use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tauri::AppHandle;
use zip::write::SimpleFileOptions;

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

#[derive(Debug, Deserialize)]
pub(crate) struct ExportZipArgs {
    pub(crate) image_ids: Vec<String>,
    pub(crate) dest_path: String,
}

#[tauri::command]
pub fn export_media_zip(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: ExportZipArgs,
) -> Result<(), AppError> {
    if args.image_ids.is_empty() {
        return Err(AppError::Invalid("no media selected".into()));
    }

    let mut entries: Vec<(String, PathBuf, String)> = Vec::new();
    {
        let conn = state.conn()?;
        for id in &args.image_ids {
            let img = session::get_image(&conn, id)?;
            let abs = paths::abs_from_rel(&app, &img.rel_path)?;
            entries.push((id.clone(), abs, img.mime));
        }
    }

    let file = File::create(&args.dest_path).map_err(|e| {
        AppError::Other(format!("create zip {}: {e}", args.dest_path))
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut used_names: HashSet<String> = HashSet::new();
    for (i, (id, abs, mime)) in entries.iter().enumerate() {
        let bytes = std::fs::read(abs).map_err(|e| {
            AppError::Other(format!("read media {id}: {e}"))
        })?;
        let name = unique_zip_name(abs, mime, id, i, &mut used_names);
        zip.start_file(&name, opts)
            .map_err(|e| AppError::Other(format!("zip start_file: {e}")))?;
        zip.write_all(&bytes)
            .map_err(|e| AppError::Other(format!("zip write: {e}")))?;
    }

    zip.finish()
        .map_err(|e| AppError::Other(format!("zip finish: {e}")))?;
    Ok(())
}

fn unique_zip_name(
    abs: &Path,
    mime: &str,
    id: &str,
    index: usize,
    used: &mut HashSet<String>,
) -> String {
    let stem = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("media");
    let ext = abs
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| mime_to_ext(mime).to_string());

    let base = format!("{stem}.{ext}");
    if used.insert(base.clone()) {
        return base;
    }

    let short = if id.len() >= 8 { &id[..8] } else { id };
    let with_id = format!("{stem}_{short}.{ext}");
    if used.insert(with_id.clone()) {
        return with_id;
    }

    let with_idx = format!("{stem}_{index}.{ext}");
    used.insert(with_idx.clone());
    with_idx
}

fn mime_to_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/png" => "png",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ if mime.starts_with("video/") => "mp4",
        _ => "bin",
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteMediaArgs {
    pub(crate) image_ids: Vec<String>,
}

#[tauri::command]
pub fn delete_media(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: DeleteMediaArgs,
) -> Result<(), AppError> {
    if args.image_ids.is_empty() {
        return Ok(());
    }
    let removed = {
        let conn = state.conn()?;
        session::delete_images(&conn, &args.image_ids)?
    };
    for (rel, thumb) in removed {
        if let Ok(abs) = paths::abs_from_rel(&app, &rel) {
            let _ = std::fs::remove_file(&abs);
        }
        if let Some(t) = thumb {
            if let Ok(abs) = paths::abs_from_rel(&app, &t) {
                let _ = std::fs::remove_file(&abs);
            }
        }
    }
    Ok(())
}
