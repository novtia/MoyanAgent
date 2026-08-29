//! In-loop [`ChatRequest::tool_chain`] windowing.
//!
//! Each agent run accumulates one [`ToolChainRound`] per tool-call cycle.
//! Unbounded growth (e.g. dozens of Read/Edit rounds) blows up the prompt
//! even when session `history` is small. We retain:
//!
//! - The **latest** TodoList-only round ??older create/update transcripts
//!   are replaced by the live [`crate::ai::chat::ChatRequest::todo_snapshot`].
//! - The **latest N** rounds for every other tool.

use crate::ai::agent::tools::todo::TOOL_NAME;
use crate::ai::chat::ToolChainRound;
use crate::ai::tokens::{estimate_tool_round_tokens, truncate_tool_content};

/// Default cap on non-TodoList tool rounds kept in the internal loop.
pub const DEFAULT_MAX_NON_TODO_TOOL_ROUNDS: usize = 10;

/// Share of the context window the in-turn tool transcript may hold.
///
/// Replayed history already claims half the window (see
/// `app::history::HISTORY_BUDGET_RATIO`); this leaves the remaining 15% for the
/// system prompt, the tool schema, the user's message and the completion
/// reservation.
pub const TOOL_CHAIN_BUDGET_RATIO: f64 = 0.35;

/// Token budget for the tool chain, derived from the model's window.
pub fn token_budget(context_window: Option<i64>) -> Option<i64> {
    context_window
        .filter(|w| *w > 0)
        .map(|w| (w as f64 * TOOL_CHAIN_BUDGET_RATIO) as i64)
}

/// True when this round only exists to service a TodoList call.
pub fn is_todo_round(round: &ToolChainRound) -> bool {
    !round.assistant.tool_calls.is_empty()
        && round
            .assistant
            .tool_calls
            .iter()
            .all(|tc| tc.name == TOOL_NAME)
}

/// Drop older TodoList-only rounds, then window the rest.
///
/// Keeping every TodoList create/update in the prompt is what made the
/// model forget unfinished items once the context grew: the live list was
/// buried under its own history. The engine injects a compact ????snapshot
/// at the end of each request instead.
///
/// Trimming non-todo rounds overshoots down to a low-water mark rather than
/// shaving off exactly one round. Rounds already sent sit in the provider's
/// context cache, and evicting the oldest on every single loop iteration
/// would shift the prompt prefix each time and forfeit those hits.
///
/// `token_budget` is the second, independent limit. A round count says nothing
/// about size: ten rounds of prose are trivial while ten rounds that each
/// returned a manuscript are worth more than the whole window. Without a token
/// limit the chain is the one part of the request that grows without bound ??
/// compaction only ever rewrites `history`.
pub fn trim_tool_chain(
    chain: &mut Vec<ToolChainRound>,
    max_non_todo: usize,
    token_budget: Option<i64>,
) {
    drop_older_todo_rounds(chain);
    trim_to_round_cap(chain, max_non_todo);
    if let Some(budget) = token_budget.filter(|b| *b > 0) {
        trim_to_token_budget(chain, budget);
    }
}

fn trim_to_round_cap(chain: &mut Vec<ToolChainRound>, max_non_todo: usize) {
    let non_todo_count = chain.iter().filter(|r| !is_todo_round(r)).count();
    if non_todo_count <= max_non_todo {
        return;
    }
    // Half the cap, but never zero: the round committed a moment ago carries
    // the tool result the model is about to answer, so it has to survive.
    let low_water = if max_non_todo == 0 {
        0
    } else {
        (max_non_todo / 2).max(1)
    };
    let mut drop_remaining = non_todo_count - low_water;
    chain.retain(|round| {
        if is_todo_round(round) {
            return true;
        }
        if drop_remaining > 0 {
            drop_remaining -= 1;
            false
        } else {
            true
        }
    });
}

/// Drop oldest-first until the chain fits `budget`; if the single surviving
/// round is itself over budget, elide the middle of its results.
///
/// The newest round always survives whole-round eviction: it holds the tool
/// output the model is about to answer, so dropping it would leave the request
/// asking about a result the model cannot see.
fn trim_to_token_budget(chain: &mut Vec<ToolChainRound>, budget: i64) {
    let mut total: i64 = chain.iter().map(estimate_tool_round_tokens).sum();
    if total <= budget {
        return;
    }

    while total > budget && chain.len() > 1 {
        let removed = estimate_tool_round_tokens(&chain.remove(0));
        total -= removed;
    }

    if total <= budget {
        return;
    }
    // One round, still too big ??a single uncapped tool result. Shrink the
    // results in place rather than dropping the round.
    if let Some(round) = chain.last_mut() {
        let results = round.results.len().max(1) as i64;
        let share = (budget / results).max(1);
        for result in round.results.iter_mut() {
            truncate_tool_content(&mut result.content, share);
        }
    }
}

/// Keep only the most recent TodoList-only round; older ones are redundant
/// once the live snapshot carries current state.
fn drop_older_todo_rounds(chain: &mut Vec<ToolChainRound>) {
    let Some(last) = chain.iter().rposition(is_todo_round) else {
        return;
    };
    let mut idx = 0;
    chain.retain(|round| {
        let keep = !is_todo_round(round) || idx == last;
        idx += 1;
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::{PendingAssistantTurn, ToolResultMessage};

    fn round_with_tool(name: &str) -> ToolChainRound {
        round_with_tool_id(name, "tc1")
    }

    fn round_with_tool_id(name: &str, id: &str) -> ToolChainRound {
        round_sized(name, id, serde_json::json!({"ok": true}))
    }

    fn round_sized(name: &str, id: &str, content: serde_json::Value) -> ToolChainRound {
        ToolChainRound {
            assistant: PendingAssistantTurn {
                text: None,
                thinking_content: None,
                tool_calls: vec![crate::ai::chat::ProviderToolCall {
                    id: id.into(),
                    name: name.into(),
                    arguments: serde_json::json!({}),
                }],
            },
            results: vec![ToolResultMessage {
                tool_call_id: id.into(),
                content,
                is_error: false,
            }],
        }
    }

    /// A round whose result is a full-manuscript `Read`: the shape that made
    /// the round cap useless on its own.
    fn fat_round(tokens: usize) -> ToolChainRound {
        round_sized(
            "Read",
            "fat",
            // 4 ASCII characters per estimated token.
            serde_json::json!({ "text": "m".repeat(tokens * 4) }),
        )
    }

    fn chain_tokens(chain: &[ToolChainRound]) -> i64 {
        chain.iter().map(estimate_tool_round_tokens).sum()
    }

    #[test]
    fn keeps_all_when_under_cap() {
        let mut chain: Vec<ToolChainRound> = (0..8)
            .map(|_| round_with_tool("Edit"))
            .collect();
        trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS, None);
        assert_eq!(chain.len(), 8);
    }

    #[test]
    fn drops_to_low_water_mark_beyond_cap() {
        let mut chain: Vec<ToolChainRound> = (0..15)
            .map(|_| round_with_tool("Edit"))
            .collect();
        trim_tool_chain(&mut chain, 10, None);
        assert_eq!(chain.len(), 5);
        for r in &chain {
            assert!(!is_todo_round(r));
        }
    }

    /// Trimming must not nibble one round per iteration: the retained prefix
    /// has to stay byte-identical for several rounds so the provider's context
    /// cache keeps hitting.
    #[test]
    fn window_start_holds_still_between_trims() {
        let mut chain: Vec<ToolChainRound> = Vec::new();
        let mut trims = 0;
        let mut previous_len = 0;
        for _ in 0..30 {
            chain.push(round_with_tool("Edit"));
            let before = chain.len();
            trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS, None);
            if chain.len() != before {
                trims += 1;
            } else if previous_len > 0 {
                assert_eq!(chain.len(), previous_len + 1, "untrimmed round must append");
            }
            previous_len = chain.len();
            assert!(chain.len() <= DEFAULT_MAX_NON_TODO_TOOL_ROUNDS);
        }
        // 30 rounds capped at 10 would trim ~20 times when shaving one at a
        // time; overshooting to the low-water mark keeps it to a handful.
        assert!(trims <= 5, "trimmed {trims} times, window is still sliding");
    }

    #[test]
    fn only_the_latest_todo_round_is_kept() {
        let mut chain = Vec::new();
        let mut last_todo_id = String::new();
        for i in 0..20 {
            if i % 3 == 0 {
                last_todo_id = format!("todo-{i}");
                chain.push(round_with_tool_id(TOOL_NAME, &last_todo_id));
            } else {
                chain.push(round_with_tool("Edit"));
            }
        }
        let todo_before = chain.iter().filter(|r| is_todo_round(r)).count();
        assert!(todo_before > 1);
        trim_tool_chain(&mut chain, 10, None);
        let todos: Vec<_> = chain.iter().filter(|r| is_todo_round(r)).collect();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].assistant.tool_calls[0].id, last_todo_id);
        let non_todo_after = chain.iter().filter(|r| !is_todo_round(r)).count();
        assert_eq!(non_todo_after, 5);
    }

    #[test]
    fn older_todo_rounds_drop_even_under_the_non_todo_cap() {
        let mut chain = vec![
            round_with_tool_id(TOOL_NAME, "todo-old"),
            round_with_tool("Edit"),
            round_with_tool_id(TOOL_NAME, "todo-new"),
        ];
        trim_tool_chain(&mut chain, 10, None);
        assert_eq!(chain.len(), 2);
        assert!(!is_todo_round(&chain[0]));
        assert!(is_todo_round(&chain[1]));
        assert_eq!(chain[1].assistant.tool_calls[0].id, "todo-new");
    }

    #[test]
    fn preserves_relative_order() {
        let mut chain = vec![
            round_with_tool("Edit"),
            round_with_tool(TOOL_NAME),
            round_with_tool("Read"),
        ];
        for _ in 0..12 {
            chain.push(round_with_tool("Edit"));
        }
        trim_tool_chain(&mut chain, 10, None);
        assert!(is_todo_round(&chain[0]));
        assert_eq!(
            chain[0].assistant.tool_calls[0].name,
            TOOL_NAME
        );
    }

    #[test]
    fn budget_is_a_third_of_the_window_and_absent_when_unknown() {
        assert_eq!(token_budget(Some(1_000_000)), Some(350_000));
        assert_eq!(token_budget(None), None);
        assert_eq!(token_budget(Some(0)), None);
    }

    /// The regression this parameter exists for: five rounds is well inside the
    /// round cap, yet each one returned a manuscript. The old signature had no
    /// way to notice.
    #[test]
    fn fat_rounds_inside_the_round_cap_are_still_trimmed() {
        let mut chain: Vec<ToolChainRound> = (0..5).map(|_| fat_round(200_000)).collect();
        assert!(chain_tokens(&chain) > 900_000, "precondition: over a window");
        trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS, Some(350_000));
        assert!(chain_tokens(&chain) <= 350_000);
        assert!(!chain.is_empty(), "the pending round must survive");
    }

    /// The newest round holds the result the model is about to answer, so it is
    /// shrunk in place rather than dropped.
    #[test]
    fn a_single_oversized_round_is_elided_not_dropped() {
        let mut chain = vec![fat_round(900_000)];
        trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS, Some(100_000));
        assert_eq!(chain.len(), 1, "the pending round must survive");
        assert!(chain_tokens(&chain) <= 100_000);
    }

    #[test]
    fn a_chain_inside_its_token_budget_is_untouched() {
        let mut chain: Vec<ToolChainRound> = (0..6).map(|_| round_with_tool("Edit")).collect();
        let before = chain_tokens(&chain);
        trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS, Some(350_000));
        assert_eq!(chain.len(), 6);
        assert_eq!(chain_tokens(&chain), before);
    }
}
