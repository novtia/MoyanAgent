//! Conversation compaction.
//!
//! When the running token budget approaches the provider's context
//! window, we ask the model to summarise the older portion of the
//! history into a single hidden meta-turn. The recent N turns are
//! preserved verbatim so the model still sees the immediate task
//! context with full fidelity.
//!
//! The compaction round-trip uses the *same provider* as the main loop
//! but with `tools = []` and a dedicated system prompt — we don't want
//! the summariser deciding to call tools.

use crate::ai::agent::exec::engine::ProviderEngine;
use crate::ai::chat::{ChatRequest, HistoryTurn};
use crate::ai::tokens::TokenUsage;
use crate::error::AppResult;

/// Tuning parameters for compaction. Defaults aim at a 128k-context
/// model with comfortable headroom for the final answer.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    /// Total token threshold (prompt + completion) that triggers
    /// compaction on the *following* turn.
    pub threshold_tokens: i64,
    /// Number of most-recent history turns to keep verbatim.
    pub keep_recent: usize,
    /// Hard cap on the summary length (words) the model is asked for.
    pub summary_max_words: u32,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            threshold_tokens: 120_000,
            keep_recent: 4,
            summary_max_words: 300,
        }
    }
}

/// Decision-only check. Returns `true` iff `chat.history` has enough
/// material to compact *and* token usage crossed the threshold.
pub fn should_compact(
    history_len: usize,
    usage: &TokenUsage,
    policy: &CompactionPolicy,
) -> bool {
    let total = usage.total_tokens.unwrap_or(0);
    total >= policy.threshold_tokens && history_len > policy.keep_recent + 1
}

/// Run a compaction pass on `chat.history` in place.
///
/// - Splits history into `[older, recent]` at `len - keep_recent`.
/// - Issues a side-channel summarisation request to the provider.
/// - Replaces `older` with a single hidden meta-turn containing the
///   summary, wrapped in `<compacted_summary>…</compacted_summary>`.
///
/// On error the chat is left untouched so the main loop can keep going.
pub async fn compact(
    chat: &mut ChatRequest,
    provider: &ProviderEngine,
    policy: &CompactionPolicy,
) -> AppResult<()> {
    if chat.history.len() <= policy.keep_recent {
        return Ok(());
    }
    let split = chat.history.len() - policy.keep_recent;
    let older: Vec<HistoryTurn> = chat.history[..split].to_vec();

    let mut summary_req = chat.clone();
    summary_req.history = older;
    summary_req.tools.clear();
    summary_req.tool_results.clear();
    summary_req.pending_assistant_turn = None;
    summary_req.attachments.clear();
    summary_req.system_prompt = "You are a context-compaction assistant. \
        Summarise the conversation above so a fresh model can pick it up. \
        Preserve: decisions made, file paths touched, errors hit, pending TODOs, \
        and any user-stated constraints. Drop pleasantries and redundant chatter."
        .to_string();
    summary_req.prompt = format!(
        "Produce a concise summary in at most {} words. Reply with the summary only.",
        policy.summary_max_words
    );

    let resp = provider.run(summary_req, None).await?;
    let summary = resp
        .text
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(empty summary)".into());

    let meta = HistoryTurn {
        role: "user".to_string(),
        text: Some(format!("<compacted_summary>\n{summary}\n</compacted_summary>")),
        images: Vec::new(),
    };

    let recent: Vec<HistoryTurn> = chat.history.split_off(split);
    chat.history.clear();
    chat.history.push(meta);
    chat.history.extend(recent);
    Ok(())
}
