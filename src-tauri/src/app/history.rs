use tauri::AppHandle;

use crate::ai::chat;
use crate::data::{db, session};
use crate::error::AppResult;
use crate::media::images;

use super::state::AppState;

pub(crate) fn build_history(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    before_ms: Option<i64>,
    max_messages: usize,
) -> AppResult<Vec<chat::HistoryTurn>> {
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
        .filter(|m| message_qualifies_for_history(m))
        .collect();

    let len = candidates.len();
    let start = len.saturating_sub(max_messages);
    let mut out: Vec<chat::HistoryTurn> = Vec::with_capacity(len - start);
    for m in &candidates[start..] {
        let want_roles: &[&str] = match m.role.as_str() {
            "user" => &["input", "edited"],
            "assistant" => &["output", "edited"],
            _ => &[],
        };
        let mut imgs: Vec<&session::ImageRef> = m
            .images
            .iter()
            .filter(|i| want_roles.contains(&i.role.as_str()) && i.mime.starts_with("image/"))
            .collect();
        imgs.sort_by_key(|i| i.ord);
        let mut payload: Vec<chat::AttachmentBytes> = Vec::with_capacity(imgs.len());
        for img in imgs {
            let bytes = images::read_image_bytes(app, img)?;
            payload.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
                media_role: img.media_role.clone(),
                source_url: img.source_url.clone(),
            });
        }
        let thinking_content = message_thinking_for_history(m);
        let timeline = message_timeline_for_history(m);
        out.push(chat::HistoryTurn {
            role: m.role.clone(),
            text: history_text_for_message(m),
            images: payload,
            thinking_content,
            timeline,
        });
    }
    Ok(out)
}

/// Load the current character state board for prompt injection: prefer the
/// in-memory store, fall back to the latest persisted snapshot.
pub(crate) fn load_roles_for_prompt(state: &AppState, session_id: &str) -> AppResult<Vec<serde_json::Value>> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, session_id)?;
    let live = state.role_states.snapshot(&scope);
    if !live.is_empty() {
        return Ok(live);
    }
    crate::data::role_state::latest_roles(&conn, &scope)
}

/// Render the role board as a tagged user-meta block appended to history.
pub(crate) fn format_role_state_history_block(roles: &[serde_json::Value]) -> String {
    let json = serde_json::to_string_pretty(roles).unwrap_or_else(|_| "[]".to_string());
    format!(
        "<role-state>\n\
         当前角色状态板（JSON）。续写正文时请与此状态保持一致；女性 nsfw.semen 的 ml 字段请按故事尺度理解。\n\n\
         {json}\n\
         </role-state>"
    )
}

/// Append the latest role board as the final history turn so the model sees
/// structured character state after the conversational transcript.
pub(crate) fn append_role_state_history_tail(
    state: &AppState,
    session_id: &str,
    history: &mut Vec<chat::HistoryTurn>,
) -> AppResult<()> {
    let roles = load_roles_for_prompt(state, session_id)?;
    if roles.is_empty() {
        return Ok(());
    }
    history.push(chat::HistoryTurn {
        role: "user".into(),
        text: Some(format_role_state_history_block(&roles)),
        images: Vec::new(),
        thinking_content: None,
        timeline: Vec::new(),
    });
    Ok(())
}

pub(crate) fn concat_block_text(blocks: &[serde_json::Value], block_type: &str) -> String {
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some(block_type))
        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

pub(crate) fn message_blocks(m: &session::Message) -> Option<&Vec<serde_json::Value>> {
    m.params
        .as_ref()
        .and_then(|p| p.get("blocks"))
        .and_then(|v| v.as_array())
}

/// Ordered timeline for a prior assistant turn, used by providers to
/// replay tool history in the native call/response format instead of a
/// leaked plain-text transcript. Prefers reconstructing from
/// `params.blocks` (so per-round thinking is preserved) and falls back
/// to the persisted `params.timeline` for rows without blocks.
pub(crate) fn message_timeline_for_history(m: &session::Message) -> Vec<chat::TimelineSegment> {
    if m.role != "assistant" {
        return Vec::new();
    }
    if let Some(blocks) = message_blocks(m) {
        let segs = crate::ai::block_timeline::restore_timeline_from_blocks(blocks);
        if !segs.is_empty() {
            return segs;
        }
    }
    if let Some(params) = m.params.as_ref() {
        if let Some(tv) = params.get("timeline") {
            if let Ok(segs) = serde_json::from_value::<Vec<chat::TimelineSegment>>(tv.clone()) {
                if !segs.is_empty() {
                    return segs;
                }
            }
        }
    }
    Vec::new()
}

/// Visible assistant/user reply text for provider history. For assistant
/// turns this is the timeline's final `Text` segment (the model's actual
/// reply); tool transcripts are NEVER folded into text anymore. Falls
/// back to the persisted `text`/block text, always cleaned of any leaked
/// host tool-log lines so legacy rows don't re-teach the model to echo
/// them.
pub(crate) fn history_text_for_message(m: &session::Message) -> Option<String> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    if m.role == "assistant" {
        let timeline = message_timeline_for_history(m);
        let summary = crate::ai::block_timeline::timeline_summary_text(&timeline);
        if !summary.is_empty() {
            return Some(summary);
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = m.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let cleaned = strip_leaked_host_tool_log(t);
        if !cleaned.trim().is_empty() {
            parts.push(cleaned.trim().to_string());
        }
    }
    if let Some(blocks) = message_blocks(m) {
        let block_text = strip_leaked_host_tool_log(concat_block_text(blocks, "text").trim());
        let block_text = block_text.trim().to_string();
        if !block_text.is_empty() && !parts.iter().any(|p| p.contains(&block_text)) {
            parts.push(block_text);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(crate) fn message_thinking_for_history(m: &session::Message) -> Option<String> {
    let mut thinking = m
        .params
        .as_ref()
        .and_then(|p| p.get("thinking_content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if let Some(blocks) = message_blocks(m) {
        let block_thinking = concat_block_text(blocks, "thinking").trim().to_string();
        if block_thinking.len() > thinking.len() {
            thinking = block_thinking;
        }
    }
    if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    }
}

pub(crate) fn message_qualifies_for_history(m: &session::Message) -> bool {
    let has_text = history_text_for_message(m).is_some();
    let has_img = m
        .images
        .iter()
        .any(|i| matches!(i.role.as_str(), "input" | "output" | "edited"));
    if m.role == "assistant" {
        return has_text
            || has_img
            || message_thinking_for_history(m).is_some()
            || !message_timeline_for_history(m).is_empty();
    }
    has_text || has_img
}
