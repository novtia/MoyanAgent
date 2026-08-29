use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::chat::{ChatRequest, HistoryTurn, TimelineSegment, ToolChainRound};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    /// Prompt tokens of the *latest* API call within an agent loop.
    ///
    /// Unlike `prompt_tokens` — which `accumulate_usage` sums across every
    /// tool-call round of a turn — this is *replaced* on each round, so it
    /// reflects the real context-window occupancy at the end of the turn (the
    /// last request already carries the full conversation history). The composer
    /// context ring uses this instead of the summed total to avoid over-counting
    /// when a turn fans out into multiple tool-call rounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_tokens: Option<i64>,
    /// Tokens served from prompt cache (OpenAI `cached_tokens`, Claude
    /// `cache_read_input_tokens`, Gemini `cachedContentTokenCount`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<i64>,
    /// Tokens written into prompt cache (Claude `cache_creation_input_tokens`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
}

impl TokenUsage {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens.is_none()
            && self.completion_tokens.is_none()
            && self.total_tokens.is_none()
            && self.cache_read_tokens.is_none()
            && self.cache_write_tokens.is_none()
    }
}

/// OpenAI-compatible `usage` object (chat completions / Responses).
pub fn extract_usage(v: &Value) -> TokenUsage {
    let usage = v.get("usage").unwrap_or(&Value::Null);
    let details = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"));
    let cache_read = details
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_i64)
        .or_else(|| usage.get("cache_read_input_tokens").and_then(Value::as_i64))
        .or_else(|| usage.get("cached_tokens").and_then(Value::as_i64));
    let cache_write = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64)
        .or_else(|| {
            details
                .and_then(|d| d.get("cache_write_tokens"))
                .and_then(Value::as_i64)
        });

    TokenUsage {
        prompt_tokens: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_i64),
        completion_tokens: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_i64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_i64),
        last_prompt_tokens: usage.get("last_prompt_tokens").and_then(Value::as_i64),
        cache_read_tokens: cache_read.filter(|n| *n > 0),
        cache_write_tokens: cache_write.filter(|n| *n > 0),
    }
}

// ─────────────────────── client-side prompt estimate ───────────────────────
//
// Providers only report `usage` *after* a call succeeds, which is useless for
// deciding whether a call may be made at all: a session restored from the
// database can already exceed the window on its very first request. These
// helpers give the agent loop a cheap, tokenizer-free approximation so it can
// compact and clamp before anything is sent.

/// Latin text averages roughly four characters per token.
const ASCII_CHARS_PER_TOKEN: i64 = 4;

/// Per-message framing (role markers, delimiters) charged by every provider.
const MESSAGE_OVERHEAD_TOKENS: i64 = 4;

/// Flat charge for an inline image. Real cost is provider- and
/// resolution-dependent (tile counts, detail level); this sits above the
/// common single-tile case so estimates stay on the safe side.
const IMAGE_TOKENS: i64 = 1_500;

/// Approximate token count of a text blob.
///
/// CJK is counted at one token per character rather than the Latin ratio —
/// tokenizers split Chinese into roughly one token per glyph, so the naive
/// `chars / 4` rule under-counts a Chinese transcript by ~4x and would let the
/// loop walk straight into a context-length rejection.
pub fn estimate_text_tokens(text: &str) -> i64 {
    let mut ascii: i64 = 0;
    let mut wide: i64 = 0;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    (ascii + ASCII_CHARS_PER_TOKEN - 1) / ASCII_CHARS_PER_TOKEN + wide
}

/// Marker left in place of elided tool output. Phrased for the model: it has to
/// understand that the content was cut for size, not that the tool failed.
fn elision_notice(dropped: i64) -> String {
    format!("\n\n…<{dropped} tokens elided to fit the context window; re-read a narrower range if you need this part>…\n\n")
}

/// Bring one tool result inside `max_tokens`, keeping both ends.
///
/// The head carries the shape of the output (headers, first matches, the
/// beginning of a document) and the tail carries where it ended up; the middle
/// is what a model can most afford to lose. Returns `true` when anything was
/// dropped.
///
/// Without this, a single uncapped tool result can exceed the whole window on
/// its own, which no amount of round-windowing or history compaction can undo.
pub fn truncate_tool_content(value: &mut Value, max_tokens: i64) -> bool {
    if max_tokens <= 0 || estimate_json_tokens(value) <= max_tokens {
        return false;
    }
    match value {
        Value::String(s) => truncate_text_to_tokens(s, max_tokens),
        Value::Array(items) => {
            let mut changed = false;
            // Charge each element an equal share rather than truncating the
            // first one to nothing: array results are usually homogeneous
            // (matches, entries) and losing whole trailing items reads as a
            // shorter result rather than a truncated one.
            let share = (max_tokens / (items.len().max(1) as i64)).max(1);
            for item in items.iter_mut() {
                changed |= truncate_tool_content(item, share);
            }
            changed
        }
        Value::Object(map) => {
            // Text fields are where the bulk lives; the scalar siblings
            // (`path`, `lines`, `truncated`) are metadata the model needs
            // intact, and they cost almost nothing.
            let bulky: Vec<String> = map
                .iter()
                .filter(|(_, v)| matches!(v, Value::String(_) | Value::Array(_) | Value::Object(_)))
                .map(|(k, _)| k.clone())
                .collect();
            if bulky.is_empty() {
                return false;
            }
            // Charge the keys and the untouchable scalars up front so the
            // shares handed to the bulky fields cannot overshoot together.
            let fixed: i64 = 2 + map
                .iter()
                .map(|(k, v)| {
                    let value_cost = if bulky.contains(k) {
                        0
                    } else {
                        estimate_json_tokens(v)
                    };
                    estimate_text_tokens(k) + 4 + value_cost
                })
                .sum::<i64>();
            let share = ((max_tokens - fixed) / bulky.len() as i64).max(1);
            let mut changed = false;
            for key in bulky {
                if let Some(v) = map.get_mut(&key) {
                    changed |= truncate_tool_content(v, share);
                }
            }
            if changed {
                map.insert("content_elided".into(), Value::Bool(true));
            }
            changed
        }
        _ => false,
    }
}

/// Middle-elide `text` so its estimate fits `max_tokens`.
fn truncate_text_to_tokens(text: &mut String, max_tokens: i64) -> bool {
    if estimate_text_tokens(text) <= max_tokens {
        return false;
    }
    let before = estimate_text_tokens(text);
    // Convert the token budget back into characters through the *cheapest*
    // ratio in the estimator (1 token per wide char), so the result is inside
    // budget whatever the script — a CJK manuscript and an ASCII log get the
    // same guarantee.
    let budget_chars = max_tokens.max(1) as usize;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= budget_chars {
        // Already short in characters yet over budget is not reachable given the
        // ratios above, but truncating to nothing would be worse than a no-op.
        return false;
    }
    let keep = budget_chars.saturating_sub(64).max(2);
    let head_len = keep / 2;
    let tail_len = keep - head_len;
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[chars.len() - tail_len..].iter().collect();
    let mut out = String::with_capacity(head.len() + tail.len() + 96);
    out.push_str(&head);
    out.push_str(&elision_notice(before - max_tokens));
    out.push_str(&tail);
    *text = out;
    true
}

fn estimate_json_tokens(value: &Value) -> i64 {
    match value {
        Value::Null => 1,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(s) => estimate_text_tokens(s) + 2,
        Value::Array(items) => 2 + items.iter().map(estimate_json_tokens).sum::<i64>(),
        Value::Object(map) => {
            2 + map
                .iter()
                .map(|(k, v)| estimate_text_tokens(k) + 2 + estimate_json_tokens(v))
                .sum::<i64>()
        }
    }
}

fn estimate_opt_text(text: Option<&String>) -> i64 {
    text.map(|s| estimate_text_tokens(s)).unwrap_or(0)
}

fn estimate_timeline_tokens(segments: &[TimelineSegment]) -> i64 {
    segments
        .iter()
        .map(|seg| match seg {
            TimelineSegment::AgentStage { .. } => 0,
            TimelineSegment::Text {
                text,
                thinking_content,
            } => {
                MESSAGE_OVERHEAD_TOKENS
                    + estimate_text_tokens(text)
                    + estimate_opt_text(thinking_content.as_ref())
            }
            TimelineSegment::ToolRound {
                assistant_text,
                thinking_content,
                calls,
                results,
            } => {
                MESSAGE_OVERHEAD_TOKENS
                    + estimate_opt_text(assistant_text.as_ref())
                    + estimate_opt_text(thinking_content.as_ref())
                    + calls
                        .iter()
                        .map(|c| {
                            MESSAGE_OVERHEAD_TOKENS
                                + estimate_text_tokens(&c.name)
                                + estimate_json_tokens(&c.arguments)
                        })
                        .sum::<i64>()
                    + results
                        .iter()
                        .map(|r| MESSAGE_OVERHEAD_TOKENS + estimate_json_tokens(&r.content))
                        .sum::<i64>()
            }
        })
        .sum()
}

/// Approximate cost of one replayed conversation turn, including the tool
/// transcript a prior assistant turn carries.
pub fn estimate_history_turn_tokens(turn: &HistoryTurn) -> i64 {
    let mut total = MESSAGE_OVERHEAD_TOKENS;
    total += estimate_opt_text(turn.text.as_ref());
    total += estimate_opt_text(turn.thinking_content.as_ref());
    total += estimate_timeline_tokens(&turn.timeline);
    total += turn.images.len() as i64 * IMAGE_TOKENS;
    total
}

/// Approximate cost of one in-turn tool round as the provider will serialise
/// it: the assistant message that emitted the calls plus every result.
pub fn estimate_tool_round_tokens(round: &ToolChainRound) -> i64 {
    let mut total = MESSAGE_OVERHEAD_TOKENS;
    total += estimate_opt_text(round.assistant.text.as_ref());
    total += estimate_opt_text(round.assistant.thinking_content.as_ref());
    for call in &round.assistant.tool_calls {
        total += MESSAGE_OVERHEAD_TOKENS
            + estimate_text_tokens(&call.name)
            + estimate_json_tokens(&call.arguments);
    }
    for result in &round.results {
        total += MESSAGE_OVERHEAD_TOKENS + estimate_json_tokens(&result.content);
    }
    total
}

/// Approximate the prompt size of a [`ChatRequest`] as the provider will
/// serialise it: system prompt, replayed history, in-turn tool rounds, the
/// tool schema, and the pending user message.
pub fn estimate_chat_tokens(chat: &ChatRequest) -> i64 {
    let mut total = MESSAGE_OVERHEAD_TOKENS + estimate_text_tokens(&chat.system_prompt);
    total += MESSAGE_OVERHEAD_TOKENS + estimate_text_tokens(&chat.prompt);
    total += chat.attachments.len() as i64 * IMAGE_TOKENS;
    total += chat
        .history
        .iter()
        .map(estimate_history_turn_tokens)
        .sum::<i64>();
    total += chat
        .tool_chain
        .iter()
        .map(estimate_tool_round_tokens)
        .sum::<i64>();
    if let Some(pending) = chat.pending_assistant_turn.as_ref() {
        total += MESSAGE_OVERHEAD_TOKENS
            + estimate_opt_text(pending.text.as_ref())
            + estimate_opt_text(pending.thinking_content.as_ref());
        for call in &pending.tool_calls {
            total += estimate_text_tokens(&call.name) + estimate_json_tokens(&call.arguments);
        }
    }
    for result in &chat.tool_results {
        total += MESSAGE_OVERHEAD_TOKENS + estimate_json_tokens(&result.content);
    }
    if let Some(snap) = chat.todo_snapshot_text() {
        total += MESSAGE_OVERHEAD_TOKENS + estimate_text_tokens(snap);
    }
    for tool in &chat.tools {
        total += estimate_text_tokens(&tool.name)
            + estimate_text_tokens(&tool.description)
            + estimate_json_tokens(&tool.schema);
    }
    total
}

#[cfg(test)]
mod estimate_tests {
    use super::*;

    #[test]
    fn latin_text_uses_the_four_char_ratio() {
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
    }

    /// A Chinese transcript must not be estimated at a quarter of its real
    /// size, or the loop happily builds a prompt four times over the window.
    #[test]
    fn cjk_is_counted_per_character() {
        assert_eq!(estimate_text_tokens("你好世界"), 4);
    }

    /// A `Read` of a long manuscript is the shape that used to blow the window
    /// on its own: one object with one enormous `text` field.
    #[test]
    fn an_oversized_read_result_is_brought_inside_budget() {
        let mut v = serde_json::json!({
            "path": "manuscript.txt",
            "lines": 12_000,
            "truncated": false,
            "text": "凌".repeat(300_000),
        });
        assert!(truncate_tool_content(&mut v, 20_000));
        assert!(estimate_json_tokens(&v) <= 20_000);
        assert_eq!(v["content_elided"], serde_json::Value::Bool(true));
        // Metadata the model reasons about must survive intact.
        assert_eq!(v["path"], "manuscript.txt");
        assert_eq!(v["lines"], 12_000);
    }

    #[test]
    fn both_ends_of_a_truncated_result_survive() {
        let body = format!("HEAD{}TAIL", "x".repeat(200_000));
        let mut v = serde_json::json!({ "text": body });
        assert!(truncate_tool_content(&mut v, 4_000));
        let text = v["text"].as_str().unwrap();
        assert!(text.starts_with("HEAD"));
        assert!(text.ends_with("TAIL"));
        assert!(text.contains("elided"));
    }

    #[test]
    fn results_inside_budget_are_untouched() {
        let mut v = serde_json::json!({ "text": "short enough" });
        assert!(!truncate_tool_content(&mut v, 10_000));
        assert_eq!(v["text"], "short enough");
        assert!(v.get("content_elided").is_none());
    }

    /// Homogeneous array output (grep matches, file entries) should shrink
    /// element-wise instead of collapsing into one truncated first item.
    #[test]
    fn array_results_shrink_element_wise() {
        let items: Vec<Value> = (0..50)
            .map(|i| Value::String(format!("match{i} {}", "z".repeat(5_000))))
            .collect();
        let mut v = Value::Array(items);
        assert!(truncate_tool_content(&mut v, 5_000));
        assert!(estimate_json_tokens(&v) <= 5_000);
        assert_eq!(v.as_array().unwrap().len(), 50);
    }

    #[test]
    fn tool_transcripts_dominate_an_assistant_turn() {
        let prose = HistoryTurn {
            role: "assistant".into(),
            text: Some("done".into()),
            images: Vec::new(),
            thinking_content: None,
            timeline: Vec::new(),
        };
        let with_tools = HistoryTurn {
            timeline: vec![TimelineSegment::ToolRound {
                assistant_text: None,
                thinking_content: None,
                calls: Vec::new(),
                results: vec![crate::ai::chat::TimelineToolResult {
                    tool_call_id: "1".into(),
                    content: Value::String("x".repeat(4_000)),
                    is_error: false,
                }],
            }],
            ..prose.clone()
        };
        assert!(estimate_history_turn_tokens(&with_tools) > 900);
        assert!(estimate_history_turn_tokens(&with_tools) > estimate_history_turn_tokens(&prose) * 10);
    }
}
