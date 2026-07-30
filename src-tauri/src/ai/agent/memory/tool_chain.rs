//! In-loop [`ChatRequest::tool_chain`] windowing.
//!
//! Each agent run accumulates one [`ToolChainRound`] per tool-call cycle.
//! Unbounded growth (e.g. dozens of Read/Edit rounds) blows up the prompt
//! even when session `history` is small. We retain:
//!
//! - **All** TodoList rounds — task state must stay visible to the model.
//! - The **latest N** rounds for every other tool.

use crate::ai::agent::tools::todo::TOOL_NAME;
use crate::ai::chat::ToolChainRound;

/// Default cap on non-TodoList tool rounds kept in the internal loop.
pub const DEFAULT_MAX_NON_TODO_TOOL_ROUNDS: usize = 10;

/// True when this round only exists to service a TodoList call.
pub fn is_todo_round(round: &ToolChainRound) -> bool {
    !round.assistant.tool_calls.is_empty()
        && round
            .assistant
            .tool_calls
            .iter()
            .all(|tc| tc.name == TOOL_NAME)
}

/// Drop the oldest non-TodoList rounds once `max_non_todo` is exceeded.
/// TodoList rounds are never removed. Order of kept rounds is preserved.
///
/// Trimming overshoots down to a low-water mark rather than shaving off exactly
/// one round. Rounds already sent sit in the provider's context cache, and
/// evicting the oldest on every single loop iteration would shift the prompt
/// prefix each time and forfeit those hits; overshooting lets the next few
/// iterations reuse an untouched prefix.
pub fn trim_tool_chain(chain: &mut Vec<ToolChainRound>, max_non_todo: usize) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::{PendingAssistantTurn, ToolResultMessage};

    fn round_with_tool(name: &str) -> ToolChainRound {
        ToolChainRound {
            assistant: PendingAssistantTurn {
                text: None,
                thinking_content: None,
                tool_calls: vec![crate::ai::chat::ProviderToolCall {
                    id: "tc1".into(),
                    name: name.into(),
                    arguments: serde_json::json!({}),
                }],
            },
            results: vec![ToolResultMessage {
                tool_call_id: "tc1".into(),
                content: serde_json::json!({"ok": true}),
                is_error: false,
            }],
        }
    }

    #[test]
    fn keeps_all_when_under_cap() {
        let mut chain: Vec<ToolChainRound> = (0..8)
            .map(|_| round_with_tool("Edit"))
            .collect();
        trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS);
        assert_eq!(chain.len(), 8);
    }

    #[test]
    fn drops_to_low_water_mark_beyond_cap() {
        let mut chain: Vec<ToolChainRound> = (0..15)
            .map(|_| round_with_tool("Edit"))
            .collect();
        trim_tool_chain(&mut chain, 10);
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
            trim_tool_chain(&mut chain, DEFAULT_MAX_NON_TODO_TOOL_ROUNDS);
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
    fn todo_rounds_are_never_dropped() {
        let mut chain = Vec::new();
        for i in 0..20 {
            if i % 3 == 0 {
                chain.push(round_with_tool(TOOL_NAME));
            } else {
                chain.push(round_with_tool("Edit"));
            }
        }
        let todo_before = chain.iter().filter(|r| is_todo_round(r)).count();
        trim_tool_chain(&mut chain, 10);
        let todo_after = chain.iter().filter(|r| is_todo_round(r)).count();
        assert_eq!(todo_before, todo_after);
        let non_todo_after = chain.iter().filter(|r| !is_todo_round(r)).count();
        assert_eq!(non_todo_after, 5);
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
        trim_tool_chain(&mut chain, 10);
        assert!(is_todo_round(&chain[0]));
        assert_eq!(
            chain[0].assistant.tool_calls[0].name,
            TOOL_NAME
        );
    }
}
