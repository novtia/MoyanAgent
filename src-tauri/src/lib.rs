mod db;
mod editor;
mod error;
mod images;
mod openrouter;
mod paths;
mod session;
mod settings;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};

pub struct AppState {
    pub pool: DbPool,
    generation_cancel: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl AppState {
    fn conn(&self) -> AppResult<crate::db::DbConn> {
        Ok(self.pool.get()?)
    }
}

fn generation_cancel_lock(
    state: &AppState,
) -> AppResult<MutexGuard<'_, HashMap<String, oneshot::Sender<()>>>> {
    state
        .generation_cancel
        .lock()
        .map_err(|_| AppError::Other("generation cancellation lock poisoned".into()))
}

fn register_generation_cancel(
    state: &AppState,
    session_id: &str,
) -> AppResult<oneshot::Receiver<()>> {
    let (tx, rx) = oneshot::channel();
    let mut guard = generation_cancel_lock(state)?;
    if guard.contains_key(session_id) {
        return Err(AppError::Invalid(
            "generation already in progress for session".into(),
        ));
    }
    guard.insert(session_id.to_string(), tx);
    Ok(rx)
}

fn clear_generation_cancel(state: &AppState, session_id: &str) {
    if let Ok(mut guard) = state.generation_cancel.lock() {
        guard.remove(session_id);
    }
}

/// Build prior-turn context for a session (oldest first), capped at `max_messages`.
/// Skips `error`-role messages and any drafts. When `before_ms` is `Some(t)`, only
/// messages with `created_at < t` are considered (used by regenerate to drop the
/// re-sent prompt and any stale assistant replies).
fn build_history(
    app: &AppHandle,
    conn: &crate::db::DbConn,
    session_id: &str,
    before_ms: Option<i64>,
    max_messages: usize,
) -> AppResult<Vec<openrouter::HistoryTurn>> {
    if max_messages == 0 {
        return Ok(Vec::new());
    }
    let loaded = session::load_with_messages(conn, session_id)?;
    let candidates: Vec<&session::Message> = loaded
        .messages
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .filter(|m| match before_ms {
            Some(t) => m.created_at < t,
            None => true,
        })
        .filter(|m| {
            // Skip empty turns (no text and no usable images) — they don't add context.
            let has_text = m.text.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
            let has_img = m.images.iter().any(|i| matches!(
                i.role.as_str(),
                "input" | "output" | "edited"
            ));
            has_text || has_img
        })
        .collect();

    let len = candidates.len();
    let start = len.saturating_sub(max_messages);
    let mut out: Vec<openrouter::HistoryTurn> = Vec::with_capacity(len - start);
    for m in &candidates[start..] {
        let want_roles: &[&str] = match m.role.as_str() {
            "user" => &["input", "edited"],
            "assistant" => &["output", "edited"],
            _ => &[],
        };
        let mut imgs: Vec<&session::ImageRef> = m
            .images
            .iter()
            .filter(|i| want_roles.contains(&i.role.as_str()))
            .collect();
        imgs.sort_by_key(|i| i.ord);
        let mut payload: Vec<openrouter::AttachmentBytes> = Vec::with_capacity(imgs.len());
        for img in imgs {
            let bytes = images::read_image_bytes(app, img)?;
            payload.push(openrouter::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
        }
        out.push(openrouter::HistoryTurn {
            role: m.role.clone(),
            text: m.text.clone(),
            images: payload,
        });
    }
    Ok(out)
}

async fn run_cancellable_generation(
    state: &AppState,
    session_id: &str,
    opts: openrouter::GenerateOptions,
) -> AppResult<openrouter::GenerateResponse> {
    let cancel_rx = register_generation_cancel(state, session_id)?;
    let result = tokio::select! {
        result = openrouter::generate(opts) => result,
        _ = cancel_rx => Err(AppError::Canceled),
    };
    clear_generation_cancel(state, session_id);
    result
}

// ───────── Settings ─────────

#[tauri::command]
fn get_settings(state: tauri::State<Arc<AppState>>) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::read(&conn)
}

#[tauri::command]
fn update_settings(
    state: tauri::State<Arc<AppState>>,
    patch: settings::SettingsPatch,
) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::apply_patch(&conn, patch)
}

// ───────── App info ─────────

#[derive(Debug, Serialize)]
struct AppInfo {
    version: String,
    data_dir: String,
    db_path: String,
    sessions_dir: String,
}

#[tauri::command]
fn get_app_info(app: AppHandle) -> Result<AppInfo, AppError> {
    let data_dir = paths::root_dir(&app)?;
    let db_path = paths::db_path(&app)?;
    let sessions_dir = paths::sessions_dir(&app)?;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        sessions_dir: sessions_dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn open_path(path: String) -> Result<(), AppError> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(AppError::NotFound(format!("path does not exist: {path}")));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(&path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn()?;
    }
    Ok(())
}

// ───────── Sessions ─────────

#[derive(Debug, Deserialize)]
struct CreateSessionArgs {
    title: Option<String>,
    model: Option<String>,
}

#[tauri::command]
fn list_sessions(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<session::SessionSummary>, AppError> {
    let conn = state.conn()?;
    session::list(&conn)
}

#[tauri::command]
fn create_session(
    state: tauri::State<Arc<AppState>>,
    args: CreateSessionArgs,
) -> Result<session::Session, AppError> {
    let conn = state.conn()?;
    session::create(&conn, args.title, args.model)
}

#[tauri::command]
fn rename_session(
    state: tauri::State<Arc<AppState>>,
    id: String,
    title: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::rename(&conn, &id, &title)
}

#[tauri::command]
fn delete_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    {
        let conn = state.conn()?;
        session::delete(&conn, &id)?;
    }
    let dir = paths::sessions_dir(&app)?.join(&id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

#[tauri::command]
fn load_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<SessionWithMessagesAbs, AppError> {
    let conn = state.conn()?;
    let s = session::load_with_messages(&conn, &id)?;
    Ok(decorate_session(&app, s))
}

#[tauri::command]
fn update_message_text(
    state: tauri::State<Arc<AppState>>,
    id: String,
    text: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::update_message_text(&conn, &id, &text)
}

#[tauri::command]
fn update_message_images(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
    image_ids: Vec<String>,
) -> Result<MessageAbs, AppError> {
    let removed = {
        let conn = state.conn()?;
        session::update_message_input_images(&conn, &id, &image_ids)?
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
    let conn = state.conn()?;
    let m = reload_message(&conn, &id)?;
    Ok(decorate_message(&app, m))
}

#[tauri::command]
fn delete_message(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    let paths = {
        let conn = state.conn()?;
        session::delete_message(&conn, &id)?
    };
    for (rel, thumb) in paths {
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

#[tauri::command]
async fn quote_message_as_attachments(
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
            if matches!(img.role.as_str(), "input" | "output" | "edited") {
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

// ───────── Attachments ─────────

#[tauri::command]
async fn add_attachment_from_path(
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
struct AttachBytesArgs {
    session_id: String,
    name: Option<String>,
    bytes: Vec<u8>,
}

#[tauri::command]
async fn add_attachment_from_bytes(
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

#[tauri::command]
fn remove_attachment_draft(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    if let Ok(abs) = paths::abs_from_rel(&app, &img.rel_path) {
        let _ = std::fs::remove_file(&abs);
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

// ───────── Image asset URL helpers ─────────

#[tauri::command]
fn get_image_abs_path(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<String, AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    let abs = paths::abs_from_rel(&app, &img.rel_path)?;
    Ok(abs.to_string_lossy().to_string())
}

// ───────── Generate ─────────

#[tauri::command]
fn cancel_generation(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<(), AppError> {
    let mut guard = generation_cancel_lock(&state)?;
    if let Some(tx) = guard.remove(&session_id) {
        let _ = tx.send(());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct GenerateReq {
    session_id: String,
    prompt: String,
    attachment_ids: Vec<String>,
    aspect_ratio: String,
    image_size: String,
}

#[derive(Debug, Serialize)]
struct GenerateResult {
    user_message: MessageAbs,
    assistant_message: MessageAbs,
}

#[tauri::command]
async fn generate_image(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    req: GenerateReq,
) -> Result<GenerateResult, AppError> {
    // 1) gather settings + attachment bytes + history synchronously
    let (opts_settings, atts_bytes, attachment_image_ids, params_json, history) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        if s.api_key.trim().is_empty() {
            return Err(AppError::Config("缺少 API Key，请先在设置中填入".into()));
        }
        if s.endpoint.trim().is_empty() {
            return Err(AppError::Config("缺少 API Endpoint".into()));
        }
        if s.model.trim().is_empty() {
            return Err(AppError::Config("缺少 Model".into()));
        }
        let mut atts: Vec<openrouter::AttachmentBytes> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for id in &req.attachment_ids {
            let img = session::get_image(&conn, id)?;
            let bytes = images::read_image_bytes(&app, &img)?;
            atts.push(openrouter::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
            ids.push(img.id.clone());
        }
        let params = serde_json::json!({
            "aspect_ratio": req.aspect_ratio,
            "image_size": req.image_size,
        });
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            None,
            s.history_turns.max(0) as usize,
        )?;
        (s, atts, ids, params.to_string(), hist)
    };

    // 2) insert user message + bind input attachments
    let user_msg = {
        let conn = state.conn()?;
        let m = session::insert_message(
            &conn,
            &req.session_id,
            "user",
            Some(req.prompt.as_str()),
            Some(params_json.as_str()),
        )?;
        session::bind_images_to_message(&conn, &m.id, &attachment_image_ids)?;
        m
    };

    // ensure session title reflects first prompt
    {
        let conn = state.conn()?;
        update_session_title_if_default(&conn, &req.session_id, &req.prompt)?;
    }

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &user_msg.id,
        }),
    );

    // 3) call OpenRouter
    let opts = openrouter::GenerateOptions {
        endpoint: opts_settings.endpoint.clone(),
        api_key: opts_settings.api_key.clone(),
        model: opts_settings.model.clone(),
        prompt: req.prompt.clone(),
        attachments: atts_bytes,
        aspect_ratio: req.aspect_ratio.clone(),
        image_size: req.image_size.clone(),
        system_prompt: opts_settings.system_prompt.clone(),
        history,
        temperature: opts_settings.temperature,
        top_p: opts_settings.top_p,
        max_tokens: opts_settings.max_tokens,
        frequency_penalty: opts_settings.frequency_penalty,
        presence_penalty: opts_settings.presence_penalty,
    };
    let result = run_cancellable_generation(&state, &req.session_id, opts).await;

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "response",
            "session_id": &req.session_id,
        }),
    );

    // 4) write assistant message
    match result {
        Ok(resp) => {
            let conn = state.conn()?;
            let assistant = session::insert_message(
                &conn,
                &req.session_id,
                "assistant",
                resp.text.as_deref(),
                Some(params_json.as_str()),
            )?;
            for (i, img) in resp.images.iter().enumerate() {
                images::write_output_image(
                    &app,
                    &conn,
                    &req.session_id,
                    &assistant.id,
                    &img.bytes,
                    &img.mime,
                    i as i64,
                )?;
            }
            // reload both messages
            let user_full = reload_message(&conn, &user_msg.id)?;
            let assistant_full = reload_message(&conn, &assistant.id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, assistant_full),
            })
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let msg_text = format!("{}", e);
            let err_msg = session::insert_message(
                &conn,
                &req.session_id,
                "error",
                Some(&msg_text),
                Some(params_json.as_str()),
            )?;
            let user_full = reload_message(&conn, &user_msg.id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, err_msg),
            })
        }
    }
}

#[derive(Debug, Deserialize)]
struct RegenerateReq {
    session_id: String,
    user_message_id: String,
    aspect_ratio: String,
    image_size: String,
}

#[tauri::command]
async fn regenerate_image(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    req: RegenerateReq,
) -> Result<GenerateResult, AppError> {
    let user_msg_existing = {
        let conn = state.conn()?;
        let m = reload_message(&conn, &req.user_message_id)?;
        if m.session_id != req.session_id {
            return Err(AppError::Invalid(
                "user_message_id does not belong to session".into(),
            ));
        }
        if m.role != "user" {
            return Err(AppError::Invalid("message must be role user".into()));
        }
        m
    };
    let prompt = user_msg_existing
        .text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::Invalid("user message has no prompt text".into()))?;

    let (opts_settings, atts_bytes, params_json, history) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        if s.api_key.trim().is_empty() {
            return Err(AppError::Config("缺少 API Key，请先在设置中填入".into()));
        }
        if s.endpoint.trim().is_empty() {
            return Err(AppError::Config("缺少 API Endpoint".into()));
        }
        if s.model.trim().is_empty() {
            return Err(AppError::Config("缺少 Model".into()));
        }
        let mut atts: Vec<openrouter::AttachmentBytes> = Vec::new();
        let mut input_images: Vec<&session::ImageRef> = user_msg_existing
            .images
            .iter()
            .filter(|i| i.role == "input")
            .collect();
        input_images.sort_by_key(|i| i.ord);
        for img in input_images {
            let bytes = images::read_image_bytes(&app, img)?;
            atts.push(openrouter::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
        }
        let params = serde_json::json!({
            "aspect_ratio": req.aspect_ratio,
            "image_size": req.image_size,
        });
        session::update_message_params(&conn, &req.user_message_id, &params.to_string())?;
        session::touch(&conn, &req.session_id)?;
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            Some(user_msg_existing.created_at),
            s.history_turns.max(0) as usize,
        )?;
        (s, atts, params.to_string(), hist)
    };

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &req.user_message_id,
        }),
    );

    let opts = openrouter::GenerateOptions {
        endpoint: opts_settings.endpoint.clone(),
        api_key: opts_settings.api_key.clone(),
        model: opts_settings.model.clone(),
        prompt: prompt.to_string(),
        attachments: atts_bytes,
        aspect_ratio: req.aspect_ratio.clone(),
        image_size: req.image_size.clone(),
        system_prompt: opts_settings.system_prompt.clone(),
        history,
        temperature: opts_settings.temperature,
        top_p: opts_settings.top_p,
        max_tokens: opts_settings.max_tokens,
        frequency_penalty: opts_settings.frequency_penalty,
        presence_penalty: opts_settings.presence_penalty,
    };
    let result = run_cancellable_generation(&state, &req.session_id, opts).await;

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "response",
            "session_id": &req.session_id,
        }),
    );

    match result {
        Ok(resp) => {
            let conn = state.conn()?;
            let assistant = session::insert_message(
                &conn,
                &req.session_id,
                "assistant",
                resp.text.as_deref(),
                Some(params_json.as_str()),
            )?;
            for (i, img) in resp.images.iter().enumerate() {
                images::write_output_image(
                    &app,
                    &conn,
                    &req.session_id,
                    &assistant.id,
                    &img.bytes,
                    &img.mime,
                    i as i64,
                )?;
            }
            let conn = state.conn()?;
            let user_full = reload_message(&conn, &req.user_message_id)?;
            let assistant_full = reload_message(&conn, &assistant.id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, assistant_full),
            })
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let msg_text = format!("{}", e);
            let err_msg = session::insert_message(
                &conn,
                &req.session_id,
                "error",
                Some(&msg_text),
                Some(params_json.as_str()),
            )?;
            let conn = state.conn()?;
            let user_full = reload_message(&conn, &req.user_message_id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, err_msg),
            })
        }
    }
}

fn update_session_title_if_default(
    conn: &crate::db::DbConn,
    id: &str,
    prompt: &str,
) -> AppResult<()> {
    let cur: Option<String> = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    if let Some(t) = cur {
        if t == "New session" || t.trim().is_empty() {
            let snippet: String = prompt.chars().take(28).collect();
            let title = if snippet.is_empty() {
                "New session".to_string()
            } else {
                snippet
            };
            conn.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2",
                rusqlite::params![title, id],
            )?;
        }
    }
    Ok(())
}

fn reload_message(conn: &crate::db::DbConn, id: &str) -> AppResult<session::Message> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, role, text, params_json, created_at FROM messages WHERE id=?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(r) = rows.next()? {
        let params_str: Option<String> = r.get(4)?;
        let mut m = session::Message {
            id: r.get(0)?,
            session_id: r.get(1)?,
            role: r.get(2)?,
            text: r.get(3)?,
            params: params_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: r.get(5)?,
            images: vec![],
        };
        let mut s = conn.prepare(
            "SELECT id, role, rel_path, thumb_path, mime, width, height, bytes, ord
             FROM message_images WHERE message_id=?1 ORDER BY ord ASC",
        )?;
        let it = s.query_map(rusqlite::params![id], |r| {
            Ok(session::ImageRef {
                id: r.get(0)?,
                role: r.get(1)?,
                rel_path: r.get(2)?,
                thumb_rel_path: r.get(3)?,
                mime: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                bytes: r.get(7)?,
                ord: r.get(8)?,
            })
        })?;
        for x in it {
            m.images.push(x?);
        }
        Ok(m)
    } else {
        Err(AppError::NotFound(format!("message {id}")))
    }
}

// ───────── Local edit ─────────

#[derive(Debug, Deserialize)]
struct EditImageArgs {
    image_id: String,
    op: editor::EditOp,
}

#[tauri::command]
fn edit_image(
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

// ───────── Export ─────────

#[derive(Debug, Deserialize)]
struct ExportArgs {
    image_id: String,
    dest_path: String,
}

#[tauri::command]
fn export_image(
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

// ───────── Decorated DTOs (with abs_path) ─────────

#[derive(Debug, Serialize)]
struct ImageRefAbs {
    id: String,
    role: String,
    rel_path: String,
    thumb_rel_path: Option<String>,
    abs_path: String,
    thumb_abs_path: Option<String>,
    mime: String,
    width: Option<i64>,
    height: Option<i64>,
    bytes: Option<i64>,
    ord: i64,
}

#[derive(Debug, Serialize)]
struct MessageAbs {
    id: String,
    session_id: String,
    role: String,
    text: Option<String>,
    params: Option<serde_json::Value>,
    created_at: i64,
    images: Vec<ImageRefAbs>,
}

#[derive(Debug, Serialize)]
struct SessionWithMessagesAbs {
    session: session::Session,
    messages: Vec<MessageAbs>,
}

fn decorate_image(app: &AppHandle, i: session::ImageRef) -> ImageRefAbs {
    let abs = paths::abs_from_rel(app, &i.rel_path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let thumb_abs = i.thumb_rel_path.as_ref().and_then(|r| {
        paths::abs_from_rel(app, r)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });
    ImageRefAbs {
        id: i.id,
        role: i.role,
        rel_path: i.rel_path,
        thumb_rel_path: i.thumb_rel_path,
        abs_path: abs,
        thumb_abs_path: thumb_abs,
        mime: i.mime,
        width: i.width,
        height: i.height,
        bytes: i.bytes,
        ord: i.ord,
    }
}

fn decorate_message(app: &AppHandle, m: session::Message) -> MessageAbs {
    MessageAbs {
        id: m.id,
        session_id: m.session_id,
        role: m.role,
        text: m.text,
        params: m.params,
        created_at: m.created_at,
        images: m
            .images
            .into_iter()
            .map(|i| decorate_image(app, i))
            .collect(),
    }
}

fn decorate_session(app: &AppHandle, s: session::SessionWithMessages) -> SessionWithMessagesAbs {
    SessionWithMessagesAbs {
        session: s.session,
        messages: s
            .messages
            .into_iter()
            .map(|m| decorate_message(app, m))
            .collect(),
    }
}

// ───────── Run ─────────

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle();
            let db_path = paths::db_path(handle)?;
            let pool = db::open_pool(&db_path)?;
            app.manage(Arc::new(AppState {
                pool,
                generation_cancel: Mutex::new(HashMap::new()),
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            get_app_info,
            open_path,
            list_sessions,
            create_session,
            rename_session,
            delete_session,
            load_session,
            delete_message,
            update_message_text,
            update_message_images,
            quote_message_as_attachments,
            add_attachment_from_path,
            add_attachment_from_bytes,
            remove_attachment_draft,
            get_image_abs_path,
            cancel_generation,
            generate_image,
            regenerate_image,
            edit_image,
            export_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
