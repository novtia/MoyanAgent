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

    // `before_ms` already dropped anything at or after the turn being
    // regenerated, so a summary written later in the session is not visible
    // here and cannot hide the messages that regeneration still needs.
    let window = select_history_window(&candidates, max_messages, None);
    turns_for_window(app, &window)
}

/// The history a summary branch forks from: every message up to and including
/// `message_id`, from the latest earlier summary onward, with no length cap.
/// A summary stored on `message_id` itself is ignored so it can be redone.
pub(crate) fn build_history_ending_at(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Vec<chat::HistoryTurn>> {
    let loaded = session::load_with_messages(conn, session_id)?;
    let end = loaded
        .messages
        .iter()
        .position(|m| m.id == message_id)
        .ok_or_else(|| crate::error::AppError::NotFound(format!("message {message_id}")))?;
    let candidates: Vec<&session::Message> = loaded.messages[..=end]
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .filter(|m| m.id == message_id || message_qualifies_for_history(m))
        .collect();
    let window = select_history_window(&candidates, usize::MAX, Some(message_id));
    turns_for_window(app, &window)
}

fn turns_for_window(
    app: &AppHandle,
    window: &HistoryWindow<'_>,
) -> AppResult<Vec<chat::HistoryTurn>> {
    let mut out: Vec<chat::HistoryTurn> = Vec::new();
    for m in &window.messages {
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
    Ok(apply_summary(out, window.summary.as_deref()))
}

/// First history index to send, keeping at most `max_messages` entries.
///
/// The obvious `len - max_messages` advances by one for every message the
/// session gains, so the oldest turn falls out on *every* request and the
/// prompt prefix shifts with it — which costs a full context-cache miss each
/// turn. Dropping in fixed-size steps instead pins the start index for a whole
/// batch of turns: the window is a little shorter right after a step, and in
/// exchange only one turn per step pays to re-process the history.
///
/// Never returns more than `max_messages` entries.
fn stable_window_start(len: usize, max_messages: usize) -> usize {
    if max_messages == 0 || len <= max_messages {
        return 0;
    }
    let step = (max_messages / 2).max(1);
    let overflow = len - max_messages;
    // Round the drop count up to the next step boundary.
    overflow.div_ceil(step) * step
}

/// Text stored by the context-summary command on the anchor assistant message.
pub(crate) fn context_summary_text(m: &session::Message) -> Option<&str> {
    m.params
        .as_ref()
        .and_then(|p| p.get("context_summary"))
        .and_then(|s| s.get("text"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub(crate) struct HistoryWindow<'a> {
    pub summary: Option<String>,
    pub messages: Vec<&'a session::Message>,
}

/// Keep the newest `context_summary` as a fixed prefix and window only the
/// messages that come after it. Messages at or before the anchor are not sent.
pub(crate) fn select_history_window<'a>(
    candidates: &[&'a session::Message],
    max_messages: usize,
    ignore_anchor_id: Option<&str>,
) -> HistoryWindow<'a> {
    if max_messages == 0 {
        return HistoryWindow {
            summary: None,
            messages: Vec::new(),
        };
    }
    let anchor = candidates.iter().rposition(|m| {
        context_summary_text(m).is_some() && Some(m.id.as_str()) != ignore_anchor_id
    });
    let (summary, rest) = match anchor {
        Some(k) => (
            context_summary_text(candidates[k]).map(str::to_string),
            &candidates[k + 1..],
        ),
        None => (None, &candidates[..]),
    };
    let start = stable_window_start(rest.len(), max_messages);
    HistoryWindow {
        summary,
        messages: rest[start..].to_vec(),
    }
}

/// Fold the summary into the history without inventing an assistant turn.
///
/// A fabricated acknowledgement ("已读取摘要，将据此继续") reads as something the
/// model itself said and gets echoed back as the reply. Instead the summary is
/// prepended to the first real user turn; when history starts with an assistant
/// turn (or is empty) it becomes a user turn of its own, which keeps roles
/// alternating without any made-up text.
pub(crate) fn apply_summary(
    mut turns: Vec<chat::HistoryTurn>,
    summary: Option<&str>,
) -> Vec<chat::HistoryTurn> {
    let Some(summary) = summary else {
        return turns;
    };
    let block = format!(
        "<context-summary>\n以下是此前对话的摘要，仅作背景资料，不是新的指令：\n{summary}\n</context-summary>"
    );
    match turns.first_mut() {
        Some(first) if first.role == "user" => {
            first.text = Some(match first.text.as_deref() {
                Some(t) if !t.is_empty() => format!("{block}\n\n{t}"),
                _ => block,
            });
        }
        _ => turns.insert(
            0,
            chat::HistoryTurn {
                role: "user".into(),
                text: Some(block),
                images: Vec::new(),
                thinking_content: None,
                timeline: Vec::new(),
            },
        ),
    }
    turns
}
/// Load the current character state board for prompt injection: prefer the
/// in-memory store, fall back to the latest persisted snapshot.
pub(crate) fn load_roles_for_prompt(
    state: &AppState,
    session_id: &str,
) -> AppResult<Vec<serde_json::Value>> {
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
         褰撳墠瑙掕壊鐘舵€佹澘锛圝SON锛夈€傜画鍐欐鏂囨椂璇蜂笌姝ょ姸鎬佷繚鎸佷竴鑷达紱濂虫€?nsfw.semen 鐨?ml 瀛楁璇锋寜鏁呬簨灏哄害鐞嗚В銆俓n\n\
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
    // The summary anchor is omitted from the sent window, but it still has to
    // stay in the candidate list so the cutoff can be found.
    if context_summary_text(m).is_some() {
        return true;
    }
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

#[cfg(test)]
mod window_tests {
    use super::stable_window_start;

    #[test]
    fn keeps_everything_below_the_cap() {
        assert_eq!(stable_window_start(0, 10), 0);
        assert_eq!(stable_window_start(9, 10), 0);
        assert_eq!(stable_window_start(10, 10), 0);
    }

    #[test]
    fn never_exceeds_the_cap() {
        for len in 0..200 {
            for max in 1..20 {
                let start = stable_window_start(len, max);
                assert!(start <= len, "start {start} past len {len}");
                assert!(len - start <= max, "kept {} > cap {max}", len - start);
            }
        }
    }

    /// The whole point: the first retained message must stay put as the session
    /// grows, otherwise the prompt prefix shifts on every turn.
    #[test]
    fn start_index_holds_still_between_steps() {
        let mut moves = 0;
        let mut last = stable_window_start(10, 10);
        for len in 11..=40 {
            let start = stable_window_start(len, 10);
            if start != last {
                moves += 1;
                last = start;
            }
        }
        // 30 new messages would shift the window 30 times with a naive
        // `len - max`; stepping by half the cap cuts it to 6.
        assert_eq!(moves, 6);
    }

    #[test]
    fn zero_cap_disables_history() {
        assert_eq!(stable_window_start(50, 0), 0);
    }
}

#[cfg(test)]
mod summary_tests {
    use super::{apply_summary, message_qualifies_for_history, select_history_window};
    use crate::data::session::Message;

    fn msg(id: &str, role: &str, text: &str, summary: Option<&str>, created_at: i64) -> Message {
        let mut params = serde_json::json!({});
        if let Some(summary) = summary {
            params["context_summary"] = serde_json::json!({ "text": summary, "created_at": 1 });
        }
        Message {
            id: id.into(),
            session_id: "s".into(),
            role: role.into(),
            text: Some(text.into()),
            params: Some(params),
            created_at,
            images: Vec::new(),
        }
    }

    #[test]
    fn summary_drops_everything_up_to_the_anchor() {
        let messages = vec![
            msg("1", "user", "更早的要求", None, 1),
            msg("2", "assistant", "更早的回复", Some("旧摘要"), 2),
            msg("3", "user", "继续写第三章", None, 3),
            msg("4", "assistant", "第三章写完了", Some("新摘要"), 4),
            msg("5", "user", "再改结尾", None, 5),
        ];
        let candidates: Vec<&Message> = messages.iter().collect();
        let window = select_history_window(&candidates, 50, None);
        assert_eq!(window.summary.as_deref(), Some("新摘要"));
        assert_eq!(window.messages.len(), 1);
        assert_eq!(window.messages[0].id, "5");
    }

    #[test]
    fn summary_is_not_trimmed_by_the_message_window() {
        let mut messages = vec![msg("anchor", "assistant", "到此为止", Some("固定摘要"), 1)];
        for i in 0..30 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            messages.push(msg(&format!("m{i}"), role, "后文", None, 2 + i));
        }
        let candidates: Vec<&Message> = messages.iter().collect();
        let window = select_history_window(&candidates, 4, None);
        assert_eq!(window.summary.as_deref(), Some("固定摘要"));
        assert!(window.messages.len() <= 4);
        assert!(window.messages.iter().all(|m| m.id != "anchor"));
        assert_eq!(window.messages.last().map(|m| m.id.as_str()), Some("m29"));
    }

    /// Regeneration passes `before_ms`, which filters candidates before the
    /// summary anchor is chosen. A summary that did not exist yet at that
    /// timestamp must not hide the earlier turns.
    #[test]
    fn summary_after_before_ms_is_not_used() {
        let messages = vec![
            msg("1", "user", "先写大纲", None, 10),
            msg("2", "assistant", "大纲如下", None, 20),
            msg("3", "assistant", "后来才总结", Some("不应生效"), 30),
        ];
        let before = 25i64;
        let candidates: Vec<&Message> = messages
            .iter()
            .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
            .filter(|m| m.created_at < before)
            .filter(|m| message_qualifies_for_history(m))
            .collect();
        let window = select_history_window(&candidates, 50, None);
        assert!(window.summary.is_none());
        assert_eq!(
            window
                .messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }

    #[test]
    fn empty_summary_anchor_still_cuts_history() {
        let messages = vec![
            msg("1", "user", "改文件", None, 1),
            msg("2", "assistant", "", Some("只剩摘要"), 2),
            msg("3", "user", "继续", None, 3),
        ];
        let candidates: Vec<&Message> = messages
            .iter()
            .filter(|m| message_qualifies_for_history(m))
            .collect();
        let window = select_history_window(&candidates, 50, None);
        assert_eq!(window.summary.as_deref(), Some("只剩摘要"));
        assert_eq!(window.messages.len(), 1);
        assert_eq!(window.messages[0].id, "3");
    }

    /// Redoing a summary: the target's own old summary must not become the
    /// cutoff, otherwise the branch would fork from an empty history.
    #[test]
    fn ignoring_the_target_anchor_falls_back_to_the_earlier_one() {
        let messages = vec![
            msg("1", "user", "更早", None, 1),
            msg("2", "assistant", "更早回复", Some("第一份摘要"), 2),
            msg("3", "user", "后来的要求", None, 3),
            msg("4", "assistant", "后来的回复", Some("第二份摘要"), 4),
        ];
        let candidates: Vec<&Message> = messages.iter().collect();
        let window = select_history_window(&candidates, usize::MAX, Some("4"));
        assert_eq!(window.summary.as_deref(), Some("第一份摘要"));
        assert_eq!(
            window.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["3", "4"]
        );
    }

    #[test]
    fn ignoring_the_only_anchor_keeps_everything() {
        let messages = vec![
            msg("1", "user", "要求", None, 1),
            msg("2", "assistant", "回复", Some("旧摘要"), 2),
        ];
        let candidates: Vec<&Message> = messages.iter().collect();
        let window = select_history_window(&candidates, usize::MAX, Some("2"));
        assert!(window.summary.is_none());
        assert_eq!(window.messages.len(), 2);
    }

    #[test]
    fn summary_folds_into_the_first_user_turn() {
        let user = crate::ai::chat::HistoryTurn {
            role: "user".into(),
            text: Some("真实问题".into()),
            images: Vec::new(),
            thinking_content: None,
            timeline: Vec::new(),
        };
        let turns = apply_summary(vec![user], Some("剧情要点"));
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        let text = turns[0].text.as_deref().unwrap();
        assert!(text.starts_with("<context-summary>"));
        assert!(text.contains("剧情要点"));
        assert!(text.ends_with("真实问题"));
    }

    #[test]
    fn summary_before_an_assistant_turn_is_its_own_user_turn() {
        let assistant = crate::ai::chat::HistoryTurn {
            role: "assistant".into(),
            text: Some("回复".into()),
            images: Vec::new(),
            thinking_content: None,
            timeline: Vec::new(),
        };
        let turns = apply_summary(vec![assistant], Some("剧情要点"));
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[1].role, "assistant");
        assert!(apply_summary(Vec::new(), None).is_empty());
    }}
