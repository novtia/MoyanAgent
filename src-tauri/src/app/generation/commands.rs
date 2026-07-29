use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::ai::agent::{FileSnapshotStore, RoleStateStore};
use crate::ai::{chat, parameters, router, session_log, token_log};
use crate::data::{db, session, settings};
use crate::error::{AppError, AppResult};
use crate::media::images;

use crate::app::dto::{decorate_message, MessageAbs};
use crate::app::history::{build_history, concat_block_text};
use crate::app::messages::reload_message;
use crate::app::reader_paths::session_project_cwd;
use crate::app::state::AppState;

use super::engine::{
    generation_abort_lock, maybe_extract_session_memory, run_agent_chain,
    run_cancellable_generation,
};
use super::params::{effective_agent_chain, resolve_session_generation};
use super::streaming::{
    new_stream_blocks, persist_streamed_assistant_snapshot, snapshot_stream_blocks,
    stream_text_callback, tool_event_callback,
};

#[tauri::command]
pub fn cancel_generation(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<(), AppError> {
    let guard = generation_abort_lock(&state)?;
    if let Some(handle) = guard.get(&session_id) {
        handle.abort();
    }
    Ok(())
}

/// Attach Volcengine Session-cache chain tip to the outgoing request, or clear
/// it when thinking settings no longer match the chain that created the tip.
fn apply_session_response_cache(
    conn: &db::DbConn,
    session_id: &str,
    session_config: &session::Session,
    chat_request: &mut chat::ChatRequest,
) -> AppResult<()> {
    if !chat_request.context_cache_enabled {
        return Ok(());
    }
    let thinking_key = parameters::thinking_cache_key(&chat_request.parameters);
    if session_config
        .cache_thinking_key
        .as_deref()
        .map(|k| k != thinking_key.as_str())
        .unwrap_or(false)
    {
        let _ = session::clear_response_cache(conn, session_id);
        chat_request.previous_response_id = None;
        return Ok(());
    }
    chat_request.previous_response_id = session_config
        .last_response_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(())
}

fn persist_session_response_cache(
    conn: &db::DbConn,
    session_id: &str,
    chat_cache_enabled: bool,
    params: &parameters::GenerationParameters,
    response_id: Option<&str>,
) {
    if !chat_cache_enabled {
        return;
    }
    let thinking_key = parameters::thinking_cache_key(params);
    let id = response_id.map(str::trim).filter(|s| !s.is_empty());
    let _ = session::set_response_cache(conn, session_id, id, Some(thinking_key.as_str()));
}

#[derive(Debug, Deserialize)]
pub(crate) struct GenerateReq {
    pub(crate) session_id: String,
    pub(crate) prompt: String,
    pub(crate) attachment_ids: Vec<String>,
    pub(crate) aspect_ratio: String,
    pub(crate) image_size: String,
    #[serde(default)]
    pub(crate) video_mode: Option<String>,
    #[serde(default)]
    pub(crate) video_duration: Option<i64>,
    #[serde(default)]
    pub(crate) video_resolution: Option<String>,
    #[serde(default)]
    pub(crate) generate_audio: Option<bool>,
    #[serde(default)]
    pub(crate) watermark: Option<bool>,
    #[serde(default)]
    pub(crate) camera_fixed: Option<bool>,
    #[serde(default)]
    pub(crate) seed: Option<i64>,
}

pub(crate) fn video_attachment_role(mode: &str, mime: &str, index: usize) -> Option<String> {
    if mime.starts_with("audio/") {
        return Some("reference_audio".into());
    }
    if mime.starts_with("video/") {
        return Some("reference_video".into());
    }
    if !mime.starts_with("image/") {
        return None;
    }
    match mode {
        "first_frame" => Some("first_frame".into()),
        "first_last" if index == 0 => Some("first_frame".into()),
        "first_last" => Some("last_frame".into()),
        "reference" => Some("reference_image".into()),
        _ => None,
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct GenerateResult {
    pub(crate) user_message: MessageAbs,
    pub(crate) assistant_message: MessageAbs,
}

/// Dedupe multimodal duplicates, persist assistant row + output images, return API DTO.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_generate_assistant_message(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    user_message_id: &str,
    params: &parameters::GenerationParameters,
    mut resp: chat::GenerateResponse,
    mut blocks: Vec<serde_json::Value>,
    role_states: &RoleStateStore,
    file_snapshots: &FileSnapshotStore,
    token_stats: &token_log::TokenStatsRecorder,
    session_logger: &session_log::SessionLogger,
    agent_type: &str,
    model: &str,
    provider: &str,
) -> AppResult<GenerateResult> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    resp.images = chat::dedupe_image_results(resp.images);
    resp.videos = chat::dedupe_media_results(resp.videos);

    // Belt-and-suspenders: scrub any leaked host tool-log lines out of the
    // persisted text blocks and the final reply before they ever hit the DB.
    for b in &mut blocks {
        if b.get("type").and_then(|v| v.as_str()) == Some("text") {
            if let Some(t) = b.get("content").and_then(|c| c.as_str()) {
                let cleaned = strip_leaked_host_tool_log(t);
                if let Some(obj) = b.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(cleaned));
                }
            }
        }
    }
    if let Some(t) = resp.text.as_deref() {
        resp.text = Some(strip_leaked_host_tool_log(t));
    }

    let block_text = concat_block_text(&blocks, "text");
    let block_thinking = concat_block_text(&blocks, "thinking");
    // Multi-turn tool loops (esp. AskUser) leave `resp.text` as only the last
    // provider beat while `blocks` hold the full interleaved transcript. Prefer
    // the longer streamed transcript so chat history keeps earlier prose.
    let prefer_blocks = !block_text.trim().is_empty()
        && block_text.trim().len()
            >= resp
                .text
                .as_ref()
                .map(|s| s.trim().len())
                .unwrap_or(0);
    if prefer_blocks
        || resp
            .text
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        if !block_text.trim().is_empty() {
            resp.text = Some(block_text.trim().to_string());
        }
    }
    if resp
        .thinking_content
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
        && !block_thinking.trim().is_empty()
    {
        resp.thinking_content = Some(block_thinking.trim().to_string());
    } else if block_thinking.len() > resp.thinking_content.as_ref().map(|s| s.len()).unwrap_or(0) {
        resp.thinking_content = Some(block_thinking.trim().to_string());
    }
    let mut assistant_params =
        params.to_assistant_message_params(&resp.usage, resp.thinking_content.as_deref());
    if !resp.videos.is_empty() {
        let metadata = resp
            .videos
            .iter()
            .map(|video| {
                serde_json::json!({
                    "mime": video.mime,
                    "width": video.width,
                    "height": video.height,
                    "duration": video.duration,
                })
            })
            .collect::<Vec<_>>();
        if let Some(obj) = assistant_params.as_object_mut() {
            obj.insert("videos".into(), serde_json::Value::Array(metadata));
        }
    }
    if !blocks.is_empty() {
        // Persist the structured timeline alongside blocks so future turns
        // replay tool history in native call/response form instead of a
        // leak-prone plain-text transcript.
        let timeline = crate::ai::block_timeline::restore_timeline_from_blocks(&blocks);
        if let Some(obj) = assistant_params.as_object_mut() {
            if !timeline.is_empty() {
                if let Ok(tv) = serde_json::to_value(&timeline) {
                    obj.insert("timeline".into(), tv);
                }
            }
            obj.insert("blocks".into(), serde_json::Value::Array(blocks));
        }
    }
    let assistant_params_json = assistant_params.to_string();
    let assistant = session::insert_message(
        conn,
        session_id,
        "assistant",
        resp.text.as_deref(),
        Some(assistant_params_json.as_str()),
    )?;
    for (i, img) in resp.images.iter().enumerate() {
        images::write_output_image(
            app,
            conn,
            session_id,
            &assistant.id,
            &img.bytes,
            &img.mime,
            i as i64,
        )?;
    }
    let video_offset = resp.images.len() as i64;
    for (i, video) in resp.videos.iter().enumerate() {
        images::write_output_video(
            app,
            conn,
            session_id,
            &assistant.id,
            &video.bytes,
            &video.mime,
            video.width,
            video.height,
            video_offset + i as i64,
        )?;
    }
    // Snapshot the character state board against this assistant message so it
    // can be re-hydrated on session open and rolled back on delete/regenerate.
    let scope_id = crate::data::role_state::resolve_role_state_scope(conn, session_id)?;
    let roles = role_states.snapshot(&scope_id);
    if !roles.is_empty() {
        let _ = crate::data::role_state::save_snapshot(
            conn,
            &scope_id,
            session_id,
            &assistant.id,
            &roles,
        );
    }

    // Bind any file mutations captured during this generation to this message
    // so they can be rolled back when it is deleted / regenerated.
    let file_changes = file_snapshots.take(session_id);
    if !file_changes.is_empty() {
        if let Err(e) = crate::data::file_snapshot::save_changes(
            conn,
            session_id,
            &assistant.id,
            &file_changes,
        ) {
            eprintln!(
                "finalize_generate: save_changes failed for session {session_id}: {e}"
            );
        }
    }
    if let Err(e) = crate::data::pending_diff::bind_message(
        conn,
        session_id,
        user_message_id,
        &assistant.id,
    ) {
        eprintln!("finalize_generate: bind_message failed for session {session_id}: {e}");
    }

    let summary_ctx = token_log::LogContext {
        session_id: Some(session_id.to_string()),
        correlation_id: Some(user_message_id.to_string()),
        agent_id: None,
        agent_type: Some(agent_type.to_string()),
    };
    token_stats.log_turn_summary(token_log::TurnSummaryLog {
        ctx: summary_ctx.clone(),
        message_id: assistant.id.clone(),
        model: model.to_string(),
        provider: provider.to_string(),
        usage: resp.usage.clone(),
    });
    session_logger.log_turn_summary(&summary_ctx, &assistant.id, model, provider, &resp.usage);

    let user_full = reload_message(conn, user_message_id)?;
    let assistant_full = reload_message(conn, &assistant.id)?;
    session::recompute_context_window_used(conn, session_id)?;
    Ok(GenerateResult {
        user_message: decorate_message(app, user_full),
        assistant_message: decorate_message(app, assistant_full),
    })
}

#[tauri::command]
pub async fn generate_image(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    req: GenerateReq,
) -> Result<GenerateResult, AppError> {
    // 1) gather settings + attachment bytes + history synchronously
    let (
        chat_request,
        params,
        attachment_image_ids,
        generation_agent,
        project_cwd,
        agent_chain,
        settings_snapshot,
        session_prompt,
        resolved_provider,
        resolved_model,
    ) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let generation_agent = session::session_generation_agent(&session_config);
        let agent_chain = effective_agent_chain(&conn, &session_config);
        let project_cwd = session_project_cwd(&conn, &req.session_id);
        // Unified parameter source: model + provider + prompt + sampling +
        // thinking all come from the session (falling back to the global
        // default only for uninitialised sessions).
        let resolved = resolve_session_generation(&conn, &s, &session_config)?;
        let session_prompt = resolved.system_prompt.clone();
        let history_turns = resolved.history_turns;
        let model_params = resolved.llm_params.clone();
        let mut atts: Vec<chat::AttachmentBytes> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for (index, id) in req.attachment_ids.iter().enumerate() {
            let img = session::get_image(&conn, id)?;
            let bytes = if img.source_url.is_some() {
                Vec::new()
            } else {
                images::read_image_bytes(&app, &img)?
            };
            let media_role = req
                .video_mode
                .as_deref()
                .and_then(|mode| video_attachment_role(mode, &img.mime, index))
                .or_else(|| img.media_role.clone());
            session::set_image_media_role(&conn, &img.id, media_role.as_deref())?;
            atts.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
                media_role,
                source_url: img.source_url.clone(),
            });
            ids.push(img.id.clone());
        }
        let mut params = parameters::factory().build(
            req.aspect_ratio.clone(),
            req.image_size.clone(),
            model_params,
        );
        if let Some(mode) = req.video_mode.as_ref() {
            params = params.with_video(
                mode.clone(),
                req.video_duration.unwrap_or(5),
                req.video_resolution
                    .clone()
                    .unwrap_or_else(|| "720p".into()),
                req.generate_audio.unwrap_or(true),
                req.watermark.unwrap_or(false),
                req.camera_fixed,
                req.seed,
            );
        }
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            None,
            history_turns.max(0) as usize,
        )?;
        let mut chat_request = router::build_chat_request(
            &resolved.provider,
            &resolved.model,
            req.prompt.clone(),
            atts,
            session_prompt.clone(),
            hist,
            params.clone(),
        )?;
        apply_session_response_cache(&conn, &req.session_id, &session_config, &mut chat_request)?;
        crate::ai::agent::exec::engine::inject_skill_cites_from_prompt(
            &app,
            &mut chat_request,
            &req.prompt,
            &s.enabled_skill_ids,
        );
        (
            chat_request,
            params,
            ids,
            generation_agent,
            project_cwd,
            agent_chain,
            s,
            session_prompt,
            resolved.provider,
            resolved.model,
        )
    };
    let params_json = params.to_message_params_json().to_string();
    let is_video_generation = chat_request.provider.sdk == crate::ai::providers::ARK_VIDEO_SDK;
    let chat_request_cache_enabled = chat_request.context_cache_enabled;

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

    // Session content log: snapshot the effective settings + the user turn
    // at the start of every generation, for debugging.
    {
        let log_ctx = token_log::LogContext {
            session_id: Some(req.session_id.clone()),
            correlation_id: Some(user_msg.id.clone()),
            agent_id: None,
            agent_type: Some(generation_agent.to_string()),
        };
        state.session_logger.log_settings(
            &log_ctx,
            serde_json::json!({
                "system_prompt": session_prompt,
                "model": chat_request.model,
                "provider": chat_request.provider.id,
                "agent_type": generation_agent,
                "agent_chain": serde_json::to_value(&agent_chain).unwrap_or(serde_json::Value::Null),
                "generation_params": serde_json::from_str::<serde_json::Value>(&params_json)
                    .unwrap_or(serde_json::Value::Null),
            }),
        );
        let attachments = attachment_image_ids
            .iter()
            .map(|id| serde_json::json!({ "image_id": id }))
            .collect();
        state
            .session_logger
            .log_user_message(&log_ctx, &req.prompt, attachments);
    }

    // ensure session title reflects first prompt
    {
        let conn = state.conn()?;
        if session_title_is_default(&conn, &req.session_id)? {
            match settings::quick_model_target(&settings_snapshot) {
                Some((provider, model_id)) => {
                    // Generate a concise title with the configured quick model
                    // off the request path so it never delays the first token.
                    let provider_cfg = chat::ProviderConfig {
                        id: provider.id.clone(),
                        name: provider.name.clone(),
                        sdk: crate::ai::providers::normalize_sdk(&provider.sdk),
                        endpoint: provider.endpoint.clone(),
                        api_key: provider.api_key.clone(),
                        context_cache_enabled: false,
                    };
                    tokio::spawn(generate_title_with_quick_model(
                        app.clone(),
                        state.inner().clone(),
                        req.session_id.clone(),
                        req.prompt.clone(),
                        provider_cfg,
                        model_id,
                    ));
                }
                None => {
                    update_session_title_if_default(&conn, &req.session_id, &req.prompt)?;
                }
            }
        }
    }

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &user_msg.id,
        }),
    );
    if is_video_generation {
        let _ = app.emit(
            "gen://status",
            serde_json::json!({
                "phase": "polling",
                "session_id": &req.session_id,
                "message_id": &user_msg.id,
            }),
        );
    }

    // 3) call the unified chat router
    let stream_blocks = new_stream_blocks();
    let on_text_delta = stream_text_callback(
        app.clone(),
        req.session_id.clone(),
        user_msg.id.clone(),
        stream_blocks.clone(),
    );
    let on_tool_event = tool_event_callback(
        app.clone(),
        req.session_id.clone(),
        user_msg.id.clone(),
        stream_blocks.clone(),
    );
    let log_model = chat_request.model.clone();
    let log_provider = chat_request.provider.id.clone();
    let result = match agent_chain
        .as_ref()
        .filter(|chain| !is_video_generation && !chain.is_empty())
    {
        Some(chain) => {
            run_agent_chain(
                &state,
                &app,
                &req.session_id,
                &user_msg.id,
                chain,
                generation_agent,
                &req.prompt,
                chat_request,
                &resolved_provider,
                &resolved_model,
                &session_prompt,
                &params,
                project_cwd,
                &stream_blocks,
                on_text_delta,
                on_tool_event,
            )
            .await
        }
        None => {
            run_cancellable_generation(
                &state,
                &req.session_id,
                generation_agent,
                req.prompt.clone(),
                chat_request,
                Some(on_text_delta),
                Some(on_tool_event),
                project_cwd,
                None,
                Some(&user_msg.id),
            )
            .await
        }
    };

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
            maybe_extract_session_memory(&state, &app, &req.session_id, &resp.usage);
            let blocks = snapshot_stream_blocks(&stream_blocks);
            let conn = state.conn()?;
            persist_session_response_cache(
                &conn,
                &req.session_id,
                chat_request_cache_enabled,
                &params,
                resp.response_id.as_deref(),
            );
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &user_msg.id,
                &params,
                resp,
                blocks,
                &state.role_states,
                &state.file_snapshots,
                &state.token_stats,
                &state.session_logger,
                generation_agent,
                &log_model,
                &log_provider,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let blocks = snapshot_stream_blocks(&stream_blocks);
            persist_streamed_assistant_snapshot(
                &conn,
                &req.session_id,
                &blocks,
                None,
                None,
                serde_json::json!({ "partial_before_error": true }),
                &state.file_snapshots,
                Some(user_msg.id.as_str()),
            )?;
            let msg_text = format!("{}", e);
            state.session_logger.log_error(
                &token_log::LogContext {
                    session_id: Some(req.session_id.clone()),
                    correlation_id: Some(user_msg.id.clone()),
                    agent_id: None,
                    agent_type: Some(generation_agent.to_string()),
                },
                &msg_text,
            );
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

/// Save partial assistant content when the user interrupts generation.
///
/// The frontend accumulates streaming text in a temporary in-memory message.
/// After cancellation, it calls this command to persist whatever was generated
/// before the interrupt so the content isn't lost on session reload.
#[tauri::command]
pub fn save_cancelled_message(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    text: String,
    thinking: String,
    blocks: Option<serde_json::Value>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let block_vec: Vec<serde_json::Value> = blocks
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.clone())
        .unwrap_or_default();
    persist_streamed_assistant_snapshot(
        &conn,
        &session_id,
        &block_vec,
        Some(text.as_str()),
        Some(thinking.as_str()),
        serde_json::json!({ "cancelled": true }),
        &state.file_snapshots,
        None,
    )
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegenerateReq {
    pub(crate) session_id: String,
    pub(crate) user_message_id: String,
    pub(crate) aspect_ratio: String,
    pub(crate) image_size: String,
    #[serde(default)]
    pub(crate) video_mode: Option<String>,
    #[serde(default)]
    pub(crate) video_duration: Option<i64>,
    #[serde(default)]
    pub(crate) video_resolution: Option<String>,
    #[serde(default)]
    pub(crate) generate_audio: Option<bool>,
    #[serde(default)]
    pub(crate) watermark: Option<bool>,
    #[serde(default)]
    pub(crate) camera_fixed: Option<bool>,
    #[serde(default)]
    pub(crate) seed: Option<i64>,
}

#[tauri::command]
pub async fn regenerate_image(
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
    let prompt = user_msg_existing.text.as_deref().unwrap_or("");
    if prompt.trim().is_empty() && user_msg_existing.images.is_empty() {
        return Err(AppError::Invalid("用户消息没有提示词或媒体附件".into()));
    }

    let (
        chat_request,
        params,
        generation_agent,
        project_cwd,
        agent_chain,
        session_prompt,
        resolved_provider,
        resolved_model,
    ) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let generation_agent = session::session_generation_agent(&session_config);
        let agent_chain = effective_agent_chain(&conn, &session_config);
        let project_cwd = session_project_cwd(&conn, &req.session_id);
        // Unified parameter source: model + provider + prompt + sampling +
        // thinking all come from the session (falling back to the global
        // default only for uninitialised sessions).
        let resolved = resolve_session_generation(&conn, &s, &session_config)?;
        let session_prompt = resolved.system_prompt.clone();
        let history_turns = resolved.history_turns;
        let model_params = resolved.llm_params.clone();
        let mut atts: Vec<chat::AttachmentBytes> = Vec::new();
        let mut input_images: Vec<&session::ImageRef> = user_msg_existing
            .images
            .iter()
            .filter(|i| i.role == "input")
            .collect();
        input_images.sort_by_key(|i| i.ord);
        for (index, img) in input_images.into_iter().enumerate() {
            let bytes = if img.source_url.is_some() {
                Vec::new()
            } else {
                images::read_image_bytes(&app, img)?
            };
            let media_role = req
                .video_mode
                .as_deref()
                .and_then(|mode| video_attachment_role(mode, &img.mime, index))
                .or_else(|| img.media_role.clone());
            session::set_image_media_role(&conn, &img.id, media_role.as_deref())?;
            atts.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
                media_role,
                source_url: img.source_url.clone(),
            });
        }
        let mut params = parameters::factory().build(
            req.aspect_ratio.clone(),
            req.image_size.clone(),
            model_params,
        );
        if let Some(mode) = req.video_mode.as_ref() {
            params = params.with_video(
                mode.clone(),
                req.video_duration.unwrap_or(5),
                req.video_resolution
                    .clone()
                    .unwrap_or_else(|| "720p".into()),
                req.generate_audio.unwrap_or(true),
                req.watermark.unwrap_or(false),
                req.camera_fixed,
                req.seed,
            );
        }
        let params_json = params.to_message_params_json().to_string();
        session::update_message_params(&conn, &req.user_message_id, &params_json)?;
        session::touch(&conn, &req.session_id)?;
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            Some(user_msg_existing.created_at),
            history_turns.max(0) as usize,
        )?;
        let mut chat_request = router::build_chat_request(
            &resolved.provider,
            &resolved.model,
            prompt.to_string(),
            atts,
            session_prompt.clone(),
            hist,
            params.clone(),
        )?;
        // Regeneration rewrites history; always start a fresh cache chain.
        let _ = session::clear_response_cache(&conn, &req.session_id);
        chat_request.previous_response_id = None;
        crate::ai::agent::exec::engine::inject_skill_cites_from_prompt(
            &app,
            &mut chat_request,
            &prompt,
            &s.enabled_skill_ids,
        );
        (
            chat_request,
            params,
            generation_agent,
            project_cwd,
            agent_chain,
            session_prompt,
            resolved.provider,
            resolved.model,
        )
    };
    let params_json = params.to_message_params_json().to_string();
    let is_video_generation = chat_request.provider.sdk == crate::ai::providers::ARK_VIDEO_SDK;
    let chat_request_cache_enabled = chat_request.context_cache_enabled;

    // Session content log: snapshot the effective settings + the (re-run)
    // user turn at the start of every regeneration, for debugging.
    {
        let log_ctx = token_log::LogContext {
            session_id: Some(req.session_id.clone()),
            correlation_id: Some(req.user_message_id.clone()),
            agent_id: None,
            agent_type: Some(generation_agent.to_string()),
        };
        state.session_logger.log_settings(
            &log_ctx,
            serde_json::json!({
                "system_prompt": session_prompt,
                "model": chat_request.model,
                "provider": chat_request.provider.id,
                "agent_type": generation_agent,
                "agent_chain": serde_json::to_value(&agent_chain).unwrap_or(serde_json::Value::Null),
                "generation_params": serde_json::from_str::<serde_json::Value>(&params_json)
                    .unwrap_or(serde_json::Value::Null),
                "regenerate": true,
            }),
        );
        let attachments = user_msg_existing
            .images
            .iter()
            .filter(|i| i.role == "input")
            .map(|i| serde_json::json!({ "image_id": i.id }))
            .collect();
        state
            .session_logger
            .log_user_message(&log_ctx, prompt, attachments);
    }

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &req.user_message_id,
        }),
    );
    if is_video_generation {
        let _ = app.emit(
            "gen://status",
            serde_json::json!({
                "phase": "polling",
                "session_id": &req.session_id,
                "message_id": &req.user_message_id,
            }),
        );
    }

    let stream_blocks = new_stream_blocks();
    let on_text_delta = stream_text_callback(
        app.clone(),
        req.session_id.clone(),
        req.user_message_id.clone(),
        stream_blocks.clone(),
    );
    let on_tool_event = tool_event_callback(
        app.clone(),
        req.session_id.clone(),
        req.user_message_id.clone(),
        stream_blocks.clone(),
    );
    let log_model = chat_request.model.clone();
    let log_provider = chat_request.provider.id.clone();
    let result = match agent_chain
        .as_ref()
        .filter(|chain| !is_video_generation && !chain.is_empty())
    {
        Some(chain) => {
            run_agent_chain(
                &state,
                &app,
                &req.session_id,
                &req.user_message_id,
                chain,
                generation_agent,
                prompt,
                chat_request,
                &resolved_provider,
                &resolved_model,
                &session_prompt,
                &params,
                project_cwd,
                &stream_blocks,
                on_text_delta,
                on_tool_event,
            )
            .await
        }
        None => {
            run_cancellable_generation(
                &state,
                &req.session_id,
                generation_agent,
                prompt.to_string(),
                chat_request,
                Some(on_text_delta),
                Some(on_tool_event),
                project_cwd,
                None,
                Some(&req.user_message_id),
            )
            .await
        }
    };

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "response",
            "session_id": &req.session_id,
        }),
    );

    match result {
        Ok(resp) => {
            maybe_extract_session_memory(&state, &app, &req.session_id, &resp.usage);
            let blocks = snapshot_stream_blocks(&stream_blocks);
            let conn = state.conn()?;
            persist_session_response_cache(
                &conn,
                &req.session_id,
                chat_request_cache_enabled,
                &params,
                resp.response_id.as_deref(),
            );
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &req.user_message_id,
                &params,
                resp,
                blocks,
                &state.role_states,
                &state.file_snapshots,
                &state.token_stats,
                &state.session_logger,
                generation_agent,
                &log_model,
                &log_provider,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let blocks = snapshot_stream_blocks(&stream_blocks);
            persist_streamed_assistant_snapshot(
                &conn,
                &req.session_id,
                &blocks,
                None,
                None,
                serde_json::json!({ "partial_before_error": true }),
                &state.file_snapshots,
                Some(req.user_message_id.as_str()),
            )?;
            let msg_text = format!("{}", e);
            state.session_logger.log_error(
                &token_log::LogContext {
                    session_id: Some(req.session_id.clone()),
                    correlation_id: Some(req.user_message_id.clone()),
                    agent_id: None,
                    agent_type: Some(generation_agent.to_string()),
                },
                &msg_text,
            );
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

/// True when the session still carries the placeholder title, i.e. it has not
/// been renamed (manually or automatically) yet.
pub(crate) fn session_title_is_default(conn: &db::DbConn, id: &str) -> AppResult<bool> {
    let cur: Option<String> = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    Ok(match cur {
        Some(t) => t == "New session" || t.trim().is_empty(),
        None => false,
    })
}

/// Normalise raw LLM output into a usable session title: keep the first line,
/// strip surrounding quotes/punctuation, and cap the length.
pub(crate) fn sanitize_generated_title(raw: &str) -> String {
    let first_line = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed = first_line.trim().trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '《'
                    | '》'
                    | '。'
                    | '.'
                    | '：'
                    | ':'
            )
    });
    trimmed.chars().take(40).collect()
}

/// Generate a session title with the configured quick model and persist it.
/// Best-effort: any failure (no response, provider error, concurrent rename)
/// silently leaves the existing title untouched.
pub(crate) async fn generate_title_with_quick_model(
    app: AppHandle,
    state: Arc<AppState>,
    session_id: String,
    prompt: String,
    provider: chat::ProviderConfig,
    model: String,
) {
    let system_prompt = "你是一个会话标题助手。请用不超过 12 个汉字（或 6 个英文单词）概括用户这条消息的主题，作为简短的会话标题。只输出标题本身，不要添加引号、标点、前缀或任何解释。".to_string();
    let request = chat::ChatRequest {
        provider,
        model,
        prompt,
        attachments: Vec::new(),
        system_prompt,
        history: Vec::new(),
        parameters: parameters::factory().build(
            "auto".into(),
            "auto".into(),
            settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
        previous_response_id: None,
        context_cache_enabled: false,
    };

    let factory = crate::ai::providers::ProviderFactory::default();
    let title = match factory.chat(request).await {
        Ok(resp) => sanitize_generated_title(&resp.text.unwrap_or_default()),
        Err(err) => {
            eprintln!("[atelier] quick-model title generation failed: {err}");
            String::new()
        }
    };
    if title.is_empty() {
        return;
    }

    let Ok(conn) = state.conn() else {
        return;
    };
    // The user (or a fallback) may have renamed the session while we waited.
    if !session_title_is_default(&conn, &session_id).unwrap_or(false) {
        return;
    }
    if session::rename(&conn, &session_id, &title).is_ok() {
        let _ = app.emit(
            "session://title",
            serde_json::json!({ "session_id": session_id, "title": title }),
        );
    }
}

pub(crate) fn update_session_title_if_default(conn: &db::DbConn, id: &str, prompt: &str) -> AppResult<()> {
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
