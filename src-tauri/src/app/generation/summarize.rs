//! Manual context summarisation, run as a branch of the conversation.
//!
//! Clicking "summarise" on an assistant message forks the conversation at that
//! message: the history is exactly what the model would see next (from the
//! latest earlier summary onward), nothing in it is rewritten, and only two
//! things differ from a normal turn — the system prompt is the summarising
//! prompt and no tools are offered. The last user message is the instruction
//! to summarise. The instruction and the reply live in a temporary child
//! session; the reply is stored on the clicked message as
//! `params.context_summary`, and [`crate::app::history::build_history`] starts
//! the model-visible history there from then on. Messages and tool cards stay
//! on screen.

use std::sync::Arc;

use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::ai::{chat, parameters, router};
use crate::app::history::build_history_ending_at;
use crate::app::messages::reload_message;
use crate::app::reader_paths::session_project_cwd;
use crate::app::state::AppState;
use crate::data::{db, session, settings};
use crate::error::{AppError, AppResult};

use super::engine::{
    generation_abort_lock, register_generation_abort, run_generation_with_signal,
};
use super::params::resolve_session_generation;
use super::streaming::{
    new_stream_blocks, persist_streamed_assistant_snapshot, snapshot_stream_blocks,
    stream_text_callback,
};

const SUMMARY_SESSION_TITLE: &str = "上下文总结";

/// The user turn that closes the branch.
const SUMMARY_INSTRUCTION: &str = "请总结以上全部对话。";

const SUMMARY_SYSTEM_PROMPT: &str = "\
你是对话上下文压缩助手。接下来的消息是用户与 AI 助手之间一段真实的历史对话，最后一条用户消息是总结指令。\
如果开头出现 <context-summary> 标签，里面是更早对话的摘要，要把它与后面的对话合并，不能丢失其中仍然有效的信息。\
请把整段对话压成一份供 AI 助手继续工作的中文摘要。\n\
要求：\n\
1. 保留：用户的目标、设定与偏好；已经做出的决定；已完成与未完成的事项；涉及的文件路径及对它们做过的改动；\
写作内容里的关键事实（人物、关系、情节走向、文风与格式约定）；用户明确提出的限制与要求。\n\
2. 不要复述项目规则、角色状态板这类每轮都会重新附带的内容。\n\
3. 不要寒暄，不要解释你在做什么，不要使用“摘要如下”之类的开场；直接用分点或短段落输出摘要正文。\n\
4. 不要续写故事，不要回答对话中尚未回答的问题，不要调用任何工具。";

/// Same agent, but with the summarising prompt and no tools.
pub(crate) fn summary_overrides() -> session::NodeOverrides {
    session::NodeOverrides {
        system_prompt: Some(SUMMARY_SYSTEM_PROMPT.to_string()),
        model: None,
        tools: Some(Vec::new()),
    }
}

/// Providers replay a turn's `timeline` as native tool_use / tool_result
/// blocks, and Claude rejects those when the request declares no tools. The
/// summary request offers none, so assistant turns fall back to their reply
/// text (`HistoryTurn::text`).
pub(crate) fn strip_timelines(history: &mut [chat::HistoryTurn]) {
    for turn in history {
        turn.timeline.clear();
    }
}

fn estimate_summary_tokens(summary: &str) -> i64 {
    ((summary.chars().count() as i64) / 2).max(1)
}

fn summary_body(resp: &chat::GenerateResponse) -> AppResult<String> {
    let split = crate::ai::rewrite_cot::split_gateway_message(
        resp.text.as_deref(),
        resp.thinking_content.as_deref(),
        None,
        None,
        None,
    );
    let body = split.body.trim();
    if body.is_empty() {
        return Err(AppError::Other("模型没有返回摘要".into()));
    }
    Ok(body.to_string())
}

fn write_summary(
    conn: &db::DbConn,
    session_id: &str,
    message_id: &str,
    summary: &str,
    child_session_id: &str,
) -> AppResult<()> {
    let message = reload_message(conn, message_id)?;
    if message.session_id != session_id || message.role != "assistant" {
        return Err(AppError::Invalid("只能总结当前会话的 AI 回复".into()));
    }
    let mut params = message.params.unwrap_or_else(|| json!({}));
    let tokens = estimate_summary_tokens(summary);
    let Some(obj) = params.as_object_mut() else {
        return Err(AppError::Other("message params are not an object".into()));
    };
    // Leave blocks, timeline, prose, and the turn's own usage in place so the
    // transcript on screen matches what was there before. Later requests omit
    // this range; the ring reads `tokens` instead of the old prompt size.
    obj.insert(
        "context_summary".into(),
        json!({
            "text": summary,
            "created_at": db::now_ms(),
            "tokens": tokens,
            "child_session_id": child_session_id,
            "status": "completed",
        }),
    );
    session::update_message_params(conn, message_id, &params.to_string())
}

/// Remember the child session after a failed run without installing a summary.
/// An empty `text` does not cut model history.
fn write_failed_summary_link(
    conn: &db::DbConn,
    session_id: &str,
    message_id: &str,
    child_session_id: &str,
) -> AppResult<()> {
    let message = reload_message(conn, message_id)?;
    if message.session_id != session_id || message.role != "assistant" {
        return Ok(());
    }
    let mut params = message.params.unwrap_or_else(|| json!({}));
    let existing = params
        .get("context_summary")
        .and_then(|s| s.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim();
    if !existing.is_empty() {
        return Ok(());
    }
    let Some(obj) = params.as_object_mut() else {
        return Ok(());
    };
    obj.insert(
        "context_summary".into(),
        json!({
            "text": "",
            "created_at": db::now_ms(),
            "child_session_id": child_session_id,
            "status": "failed",
        }),
    );
    session::update_message_params(conn, message_id, &params.to_string())
}

fn emit_summary_session(
    app: &AppHandle,
    parent_session_id: &str,
    message_id: &str,
    child_session_id: &str,
    status: &str,
) {
    let _ = app.emit(
        "summary://session",
        json!({
            "parent_session_id": parent_session_id,
            "message_id": message_id,
            "child_session_id": child_session_id,
            "status": status,
        }),
    );
}

fn emit_child_phase(app: &AppHandle, child_session_id: &str, phase: &str, message_id: &str) {
    let _ = app.emit(
        "gen://status",
        json!({
            "phase": phase,
            "session_id": child_session_id,
            "message_id": message_id,
        }),
    );
}

/// Drop the child session's abort registration when summarisation finishes.
struct ChildAbortGuard<'a> {
    state: &'a AppState,
    session_id: String,
}

impl Drop for ChildAbortGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.generation_abort.lock() {
            guard.remove(&self.session_id);
        }
    }
}

#[tauri::command]
pub async fn summarize_context(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    session_id: String,
    message_id: String,
) -> Result<(), AppError> {
    let (request, agent_type, project_cwd) = {
        let conn = state.conn()?;
        let settings = settings::read(&conn)?;
        let loaded = session::load_with_messages(&conn, &session_id)?;
        let target = loaded
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .ok_or_else(|| AppError::NotFound(format!("message {message_id}")))?;
        if target.role != "assistant" {
            return Err(AppError::Invalid("只能总结 AI 回复".into()));
        }
        let mut history = build_history_ending_at(&app, &conn, &session_id, &message_id)?;
        strip_timelines(&mut history);
        let resolved = resolve_session_generation(&conn, &settings, &loaded.session)?;
        let mut request = router::build_chat_request(
            &resolved.provider,
            &resolved.model,
            SUMMARY_INSTRUCTION.to_string(),
            Vec::new(),
            String::new(),
            history,
            parameters::factory().build("auto".into(), "auto".into(), resolved.llm_params.clone()),
        )?;
        request.context_window = Some(resolved.context_window);
        request.context_cache_enabled = false;
        request.provider.context_cache_enabled = false;
        request.previous_response_id = None;
        (
            request,
            session::session_generation_agent(&loaded.session),
            session_project_cwd(&conn, &session_id),
        )
    };

    let abort = register_generation_abort(state.inner().as_ref(), &session_id)?;
    let signal = abort.signal().clone();
    let child_id = {
        let conn = state.conn()?;
        session::create_temp(&conn, &session_id, SUMMARY_SESSION_TITLE, None)?.id
    };
    {
        let mut slots = generation_abort_lock(state.inner().as_ref())?;
        if slots.contains_key(&child_id) {
            return Err(AppError::Invalid(
                "generation already in progress for session".into(),
            ));
        }
        slots.insert(child_id.clone(), signal.controller());
    }
    let _child_abort = ChildAbortGuard {
        state: state.inner().as_ref(),
        session_id: child_id.clone(),
    };

    let user_id = {
        let conn = state.conn()?;
        let params = json!({ "spawned_prompt": true }).to_string();
        session::insert_message(
            &conn,
            &child_id,
            "user",
            Some(SUMMARY_INSTRUCTION),
            Some(params.as_str()),
        )?
        .id
    };
    emit_summary_session(&app, &session_id, &message_id, &child_id, "running");
    emit_child_phase(&app, &child_id, "request", &user_id);

    let blocks = new_stream_blocks();
    let on_text = stream_text_callback(
        app.clone(),
        child_id.clone(),
        user_id.clone(),
        blocks.clone(),
    );
    let overrides = summary_overrides();
    let result = run_generation_with_signal(
        state.inner().as_ref(),
        &session_id,
        agent_type,
        SUMMARY_INSTRUCTION.to_string(),
        request,
        Some(on_text),
        None,
        project_cwd,
        Some(&overrides),
        Some(&user_id),
        signal,
    )
    .await;

    let conn = state.conn()?;
    let snapshot = snapshot_stream_blocks(&blocks);
    let summary = match result.and_then(|resp| summary_body(&resp)) {
        Ok(text) => text,
        Err(err) => {
            let _ = persist_streamed_assistant_snapshot(
                &conn,
                &child_id,
                &snapshot,
                None,
                None,
                json!({ "cancelled": true }),
                Some(user_id.as_str()),
            );
            emit_child_phase(&app, &child_id, "response", &user_id);
            emit_summary_session(&app, &session_id, &message_id, &child_id, "failed");
            let _ = write_failed_summary_link(&conn, &session_id, &message_id, &child_id);
            return Err(err);
        }
    };

    persist_streamed_assistant_snapshot(
        &conn,
        &child_id,
        &snapshot,
        Some(summary.as_str()),
        None,
        json!({}),
        Some(user_id.as_str()),
    )?;
    emit_child_phase(&app, &child_id, "response", &user_id);
    write_summary(&conn, &session_id, &message_id, summary.trim(), &child_id)?;
    session::clear_response_cache(&conn, &session_id)?;
    session::recompute_context_window_used(&conn, &session_id)?;
    emit_summary_session(&app, &session_id, &message_id, &child_id, "completed");
    drop(abort);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, text: &str, timeline: Vec<chat::TimelineSegment>) -> chat::HistoryTurn {
        chat::HistoryTurn {
            role: role.into(),
            text: Some(text.into()),
            images: Vec::new(),
            thinking_content: None,
            timeline,
        }
    }

    #[test]
    fn overrides_swap_the_prompt_and_offer_no_tools() {
        let ov = summary_overrides();
        assert_eq!(ov.system_prompt.as_deref(), Some(SUMMARY_SYSTEM_PROMPT));
        assert_eq!(ov.tools, Some(Vec::new()));
        assert!(ov.model.is_none());
    }

    #[test]
    fn stripping_timelines_keeps_roles_and_text() {
        let mut history = vec![
            turn("user", "请改大纲", Vec::new()),
            turn(
                "assistant",
                "已改好",
                vec![chat::TimelineSegment::Text {
                    text: "已改好".into(),
                    thinking_content: None,
                }],
            ),
        ];
        strip_timelines(&mut history);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].text.as_deref(), Some("请改大纲"));
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].text.as_deref(), Some("已改好"));
        assert!(history.iter().all(|t| t.timeline.is_empty()));
    }

    #[test]
    fn summary_body_drops_thinking_and_rejects_empty() {
        let mut resp = chat::GenerateResponse {
            images: Vec::new(),
            videos: Vec::new(),
            text: Some("<think>想一想</think>最终摘要".into()),
            thinking_content: None,
            usage: Default::default(),
            tool_calls: Vec::new(),
            response_id: None,
        };
        assert_eq!(summary_body(&resp).unwrap(), "最终摘要");
        resp.text = Some("   ".into());
        assert!(summary_body(&resp).is_err());
    }
}
