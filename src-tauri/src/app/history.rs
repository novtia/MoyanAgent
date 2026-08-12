use tauri::AppHandle;

use crate::ai::chat;
use crate::data::{db, session};
use crate::error::AppResult;
use crate::media::images;

use super::state::AppState;

/// Share of the model's context window that replayed history may occupy.
///
/// The remainder covers the system prompt, injected rules, the tool schema,
/// the user's new message and the completion reservation. In-session
/// compaction cannot help here: it rewrites the in-memory request only, and
/// every new user message rebuilds the history from these rows again.
const HISTORY_BUDGET_RATIO: f64 = 0.5;

/// Token budget for replayed history, derived from the session's context
/// window. `None` (unknown window) means no token-based trimming.
pub(crate) fn history_token_budget(context_window: Option<i64>) -> Option<i64> {
    context_window
        .filter(|w| *w > 0)
        .map(|w| (w as f64 * HISTORY_BUDGET_RATIO) as i64)
}

pub(crate) fn build_history(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    before_ms: Option<i64>,
    max_messages: usize,
    token_budget: Option<i64>,
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
    let start = stable_window_start(len, max_messages);
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
    if let Some(budget) = token_budget {
        trim_history_to_budget(&mut out, budget);
    }
    Ok(out)
}

/// Bring replayed history inside `budget` estimated tokens.
///
/// `max_messages` bounds the *count* of turns, not their size — a single
/// assistant turn carrying a long tool transcript can be worth more than the
/// whole rest of the window. Tool rounds go first, oldest turn first, because
/// they are bulky and their conclusions are already restated in the prose
/// reply; only if that is not enough do whole turns get dropped.
fn trim_history_to_budget(turns: &mut Vec<chat::HistoryTurn>, budget: i64) {
    use crate::ai::tokens::estimate_history_turn_tokens;

    if budget <= 0 {
        return;
    }
    let mut total: i64 = turns.iter().map(estimate_history_turn_tokens).sum();
    if total <= budget {
        return;
    }

    for turn in turns.iter_mut() {
        if total <= budget {
            return;
        }
        let has_tool_rounds = turn
            .timeline
            .iter()
            .any(|s| matches!(s, chat::TimelineSegment::ToolRound { .. }));
        if !has_tool_rounds {
            continue;
        }
        let before = estimate_history_turn_tokens(turn);
        turn.timeline
            .retain(|s| matches!(s, chat::TimelineSegment::Text { .. }));
        total -= before - estimate_history_turn_tokens(turn);
    }

    // Keep the most recent turn no matter what: dropping it would strip the
    // context the user's new message is replying to.
    let mut drop_count = 0usize;
    while total > budget && drop_count + 1 < turns.len() {
        total -= estimate_history_turn_tokens(&turns[drop_count]);
        drop_count += 1;
    }
    turns.drain(..drop_count);
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
mod budget_tests {
    use super::{history_token_budget, trim_history_to_budget};
    use crate::ai::chat::{HistoryTurn, TimelineSegment, TimelineToolResult};
    use crate::ai::tokens::estimate_history_turn_tokens;

    fn tool_turn(prose: &str, tool_output_chars: usize) -> HistoryTurn {
        HistoryTurn {
            role: "assistant".into(),
            text: Some(prose.into()),
            images: Vec::new(),
            thinking_content: None,
            timeline: vec![
                TimelineSegment::ToolRound {
                    assistant_text: None,
                    thinking_content: None,
                    calls: Vec::new(),
                    results: vec![TimelineToolResult {
                        tool_call_id: "1".into(),
                        content: serde_json::Value::String("z".repeat(tool_output_chars)),
                        is_error: false,
                    }],
                },
                TimelineSegment::Text {
                    text: prose.into(),
                    thinking_content: None,
                },
            ],
        }
    }

    fn total(turns: &[HistoryTurn]) -> i64 {
        turns.iter().map(estimate_history_turn_tokens).sum()
    }

    #[test]
    fn budget_is_half_the_window_and_absent_when_unknown() {
        assert_eq!(history_token_budget(Some(1_000_000)), Some(500_000));
        assert_eq!(history_token_budget(None), None);
        assert_eq!(history_token_budget(Some(0)), None);
    }

    #[test]
    fn history_inside_the_budget_is_untouched() {
        let mut turns = vec![tool_turn("ok", 40)];
        let before = turns.clone();
        trim_history_to_budget(&mut turns, 10_000);
        assert_eq!(turns.len(), before.len());
        assert_eq!(turns[0].timeline.len(), before[0].timeline.len());
    }

    /// Ten messages of prose fit anywhere; ten messages each dragging a large
    /// tool transcript are what actually blow the window.
    #[test]
    fn tool_transcripts_are_dropped_before_whole_turns() {
        let mut turns: Vec<HistoryTurn> = (0..10).map(|_| tool_turn("summary", 40_000)).collect();
        trim_history_to_budget(&mut turns, 5_000);
        assert_eq!(turns.len(), 10, "prose turns should survive");
        assert!(total(&turns) <= 5_000);
        assert!(turns
            .iter()
            .all(|t| t.timeline.iter().all(|s| matches!(s, TimelineSegment::Text { .. }))));
    }

    #[test]
    fn oldest_turns_go_when_trimming_tools_is_not_enough() {
        let mut turns: Vec<HistoryTurn> = (0..10)
            .map(|i| HistoryTurn {
                role: "user".into(),
                text: Some(format!("turn{i} {}", "w".repeat(20_000))),
                images: Vec::new(),
                thinking_content: None,
                timeline: Vec::new(),
            })
            .collect();
        trim_history_to_budget(&mut turns, 10_000);
        assert!(total(&turns) <= 10_000);
        assert!(turns.len() < 10);
        assert!(turns.last().unwrap().text.as_ref().unwrap().contains("turn9"));
    }

    /// Whatever the budget, the turn the user is replying to must stay.
    #[test]
    fn the_most_recent_turn_always_survives() {
        let mut turns: Vec<HistoryTurn> = (0..3)
            .map(|_| HistoryTurn {
                role: "user".into(),
                text: Some("q".repeat(100_000)),
                images: Vec::new(),
                thinking_content: None,
                timeline: Vec::new(),
            })
            .collect();
        trim_history_to_budget(&mut turns, 10);
        assert_eq!(turns.len(), 1);
    }
}
