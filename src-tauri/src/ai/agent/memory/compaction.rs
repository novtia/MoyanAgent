//! Conversation compaction and context-budget enforcement.
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
//!
//! Occupancy is judged from two sources: the `prompt_tokens` the provider
//! reported for the previous call, and a tokenizer-free estimate of the
//! request about to be sent. The estimate is what makes compaction usable on
//! the *first* call of a turn, where no usage has been reported yet and the
//! history was just rebuilt from the database.

use crate::ai::agent::exec::engine::ProviderEngine;
use crate::ai::agent::memory::tool_chain;
use crate::ai::chat::{ChatRequest, HistoryTurn, TimelineSegment};
use crate::ai::tokens::{
    estimate_chat_tokens, estimate_text_tokens, truncate_tool_content, TokenUsage,
};
use crate::error::AppResult;

/// Fraction of the context window that may be occupied by the prompt before
/// compaction kicks in.
const DEFAULT_OCCUPANCY_RATIO: f64 = 0.7;

/// Never compact below this, however small the advertised window: a threshold
/// of a few hundred tokens would summarise on every single turn.
const MIN_DYNAMIC_THRESHOLD: i64 = 8_000;

/// Slack kept between the projected request size and the hard window limit.
/// Absorbs the estimator's error (it approximates a real tokenizer) plus
/// provider-side framing we cannot see: the tool schema's JSON envelope, role
/// delimiters, and reasoning content some providers replay back into the
/// prompt. A tenth proved too tight against real rejections.
const BUDGET_HEADROOM_RATIO: f64 = 0.15;

/// Largest share of the request budget one tool result may claim.
///
/// A single uncapped result — a `Read` of a whole manuscript, say — can exceed
/// the window by itself, and no amount of round-windowing or history
/// compaction can undo that. Capping each result keeps one call from starving
/// everything else out of the prompt.
const MAX_SINGLE_RESULT_RATIO: f64 = 0.25;

/// Floor for a clamped `max_tokens`. Below this the reply is too short to be
/// worth sending, so we let the provider reject the request instead of
/// silently producing a truncated answer.
const MIN_COMPLETION_TOKENS: i64 = 1_024;

/// Upper bound on the transcript handed to the summariser.
///
/// The whole point of compaction is that the older history no longer fits, so
/// replaying it verbatim into a side-channel call would just reproduce the
/// overflow we are trying to escape.
const SUMMARY_INPUT_MAX_TOKENS: i64 = 30_000;

/// Per-entry cap when rendering the older history for the summariser. Tool
/// output is the usual reason a turn is huge and the least worth quoting in
/// full to a summariser.
const SUMMARY_ENTRY_MAX_CHARS: usize = 2_000;

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
    /// Context window of the model actually being called, when known.
    ///
    /// Without it the fixed `threshold_tokens` is the only guard, which
    /// silently over-runs any model whose window is smaller than the default.
    pub context_window: Option<i64>,
    /// Share of `context_window` the prompt may occupy before compacting.
    pub occupancy_ratio: f64,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            threshold_tokens: 120_000,
            keep_recent: 4,
            summary_max_words: 300,
            context_window: None,
            occupancy_ratio: DEFAULT_OCCUPANCY_RATIO,
        }
    }
}

impl CompactionPolicy {
    pub fn with_context_window(mut self, context_window: Option<i64>) -> Self {
        self.context_window = context_window.filter(|c| *c > 0);
        self
    }

    /// Occupancy at which compaction should run for this model.
    ///
    /// The fixed default and the window-derived value are combined with `min`
    /// so a small model compacts early enough to stay inside its window, while
    /// a million-token model does not wait until it is holding a million
    /// tokens of transcript.
    pub fn effective_threshold(&self) -> i64 {
        match self.context_window {
            Some(window) if window > 0 => {
                let dynamic = (window as f64 * self.occupancy_ratio) as i64;
                self.threshold_tokens
                    .min(dynamic)
                    .max(MIN_DYNAMIC_THRESHOLD)
            }
            _ => self.threshold_tokens,
        }
    }

    /// Largest `prompt + max_tokens` this policy will let out the door.
    /// `None` when the window is unknown and no enforcement is possible.
    pub fn request_budget(&self) -> Option<i64> {
        self.context_window
            .filter(|w| *w > 0)
            .map(|w| (w as f64 * (1.0 - BUDGET_HEADROOM_RATIO)) as i64)
    }
}

/// Prompt estimate plus the completion tokens the provider will reserve.
///
/// DeepSeek (and most OpenAI-compatible upstreams) validate the *sum*: a
/// request with 982k of messages and `max_tokens: 200000` is rejected against
/// a 1M window even though the messages alone would fit.
pub fn projected_request_tokens(chat: &ChatRequest) -> i64 {
    estimate_chat_tokens(chat) + chat.parameters.model.max_tokens.unwrap_or(0)
}

/// Whether the request as currently built would overflow the model's window.
pub fn exceeds_budget(chat: &ChatRequest, policy: &CompactionPolicy) -> bool {
    match policy.request_budget() {
        Some(budget) => projected_request_tokens(chat) > budget,
        None => false,
    }
}

/// Decision-only check against the request that is *about* to be sent.
/// Returns `true` iff `chat.history` has enough material to compact *and* the
/// context window is close to full.
///
/// Occupancy is the larger of two readings:
///
/// - `last_prompt_tokens`, the prompt size of the most recent API call, which
///   already carries the whole conversation. `total_tokens` is only a
///   fallback: it *sums* every tool-call round of the turn, so a loop of four
///   30k rounds reports 120k while the real context is still only 30k, and
///   triggering on that would compact far too early and needlessly rewrite a
///   cached prefix.
/// - the local estimate, which covers what reported usage cannot: the first
///   call of a turn, whose history was just rebuilt from the database and may
///   already sit far past the window.
pub fn should_summarise_history(
    chat: &ChatRequest,
    usage: &TokenUsage,
    policy: &CompactionPolicy,
) -> bool {
    if chat.history.len() <= policy.keep_recent + 1 {
        return false;
    }
    let reported = usage.last_prompt_tokens.or(usage.total_tokens).unwrap_or(0);
    let occupancy = reported.max(estimate_chat_tokens(chat));
    occupancy >= policy.effective_threshold() || exceeds_budget(chat, policy)
}

/// Bring the request inside the model's budget, by force if necessary.
///
/// This is the guard that has to hold when summarisation cannot help: it has no
/// structural preconditions, needs no provider call, and runs on every turn.
/// That matters because the fastest-growing part of a long agent turn is
/// in-turn tool output, and [`compact`] only ever rewrites `history` —
/// a run whose history is too short to summarise used to sail past every check
/// while its `tool_chain` grew without bound.
///
/// Steps escalate in how much they cost the model:
///
/// 1. shrink the completion reservation (lossless — nothing leaves the prompt)
/// 2. window the in-turn tool chain by tokens
/// 3. elide the middle of any single oversized tool result
/// 4. drop the oldest history turns outright
///
/// Returns `true` when the request is inside budget afterwards. `false` means
/// every lever has been pulled and the request is still too big, which is worth
/// logging but not worth blocking on — the upstream's own rejection carries the
/// real numbers.
pub fn enforce_request_budget(chat: &mut ChatRequest, policy: &CompactionPolicy) -> bool {
    let Some(budget) = policy.request_budget() else {
        // Unreachable in normal operation: the window is resolved to a concrete
        // default before the request is built. Kept honest for direct callers.
        return true;
    };

    // The prompt has to leave at least a floor's worth of room for the reply.
    // Past that the completion reservation is negotiable and the prompt is not,
    // so the shrink steps aim at the prompt and the reservation is sized once,
    // afterwards, against whatever the prompt ended up costing. Clamping first
    // would spend the reservation down to its floor and then have no way to
    // hand back the room the shrinking freed.
    let prompt_target = (budget - MIN_COMPLETION_TOKENS).max(1);
    let configured = chat.parameters.model.max_tokens;

    if estimate_chat_tokens(chat) > prompt_target {
        let before = estimate_chat_tokens(chat);

        tool_chain::trim_tool_chain(
            &mut chat.tool_chain,
            tool_chain::DEFAULT_MAX_NON_TODO_TOOL_ROUNDS,
            tool_chain::token_budget(policy.context_window),
        );

        if estimate_chat_tokens(chat) > prompt_target {
            cap_oversized_results(chat, (budget as f64 * MAX_SINGLE_RESULT_RATIO) as i64);
        }

        if estimate_chat_tokens(chat) > prompt_target && chat.history.len() > policy.keep_recent {
            // Summarising would be gentler, but it needs a provider round-trip
            // the caller may already have spent — or already have had fail.
            // Dropping is the only lever left that is guaranteed to shrink.
            let split = user_aligned_split(&chat.history, chat.history.len() - policy.keep_recent);
            if split > 0 {
                chat.history.drain(..split);
                drop_response_cache_chain(chat);
            }
        }

        let after = estimate_chat_tokens(chat);
        if after > prompt_target {
            eprintln!(
                "[atelier] prompt still over budget after enforcement: \
                 {after} estimated vs {prompt_target} allowed (was {before})"
            );
        }
    }

    chat.parameters.model.max_tokens = configured;
    clamp_completion_budget(chat, policy);
    !exceeds_budget(chat, policy)
}

/// Elide the middle of any tool result larger than `max_per_result`.
///
/// Covers both the results already committed to the chain and the ones staged
/// for the next request, since the pending round is exactly the one carrying a
/// fresh, uncapped read.
fn cap_oversized_results(chat: &mut ChatRequest, max_per_result: i64) {
    if max_per_result <= 0 {
        return;
    }
    for result in &mut chat.tool_results {
        truncate_tool_content(&mut result.content, max_per_result);
    }
    for round in &mut chat.tool_chain {
        for result in &mut round.results {
            truncate_tool_content(&mut result.content, max_per_result);
        }
    }
}

/// Shrink `max_tokens` so the completion reservation still fits beside the
/// prompt.
///
/// Providers count the reservation against the window, so an oversized
/// `max_tokens` rejects requests whose messages would otherwise have fit. The
/// user-configured value is treated as a ceiling, never raised.
pub fn clamp_completion_budget(chat: &mut ChatRequest, policy: &CompactionPolicy) {
    let Some(budget) = policy.request_budget() else {
        return;
    };
    let Some(requested) = chat.parameters.model.max_tokens else {
        return;
    };
    if requested <= MIN_COMPLETION_TOKENS {
        return;
    }
    let room = budget - estimate_chat_tokens(chat);
    let allowed = room.max(MIN_COMPLETION_TOKENS).min(requested);
    if allowed < requested {
        chat.parameters.model.max_tokens = Some(allowed);
    }
}

/// Run a compaction pass on `chat.history` in place.
///
/// - Splits history into `[older, recent]` at `len - keep_recent`.
/// - Issues a side-channel summarisation request to the provider, seeded with
///   a truncated rendering of the older turns rather than the turns
///   themselves.
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
    // Capture InvokedSkill reminders before older turns are discarded so we
    // can reinstate them after compaction (skills must survive summaries).
    let skill_reminders = extract_invoked_skill_turns(&older);
    // The summariser sees a bounded transcript, not the raw turns: the older
    // history is over-budget by definition, so re-sending it would fail the
    // exact same way the main call is about to.
    summary_req.history.clear();
    summary_req.tools.clear();
    summary_req.tool_chain.clear();
    summary_req.tool_results.clear();
    summary_req.pending_assistant_turn = None;
    summary_req.attachments.clear();
    summary_req.previous_response_id = None;
    summary_req.todo_snapshot = None;
    summary_req.parameters.model.max_tokens = None;
    summary_req.system_prompt = "You are a context-compaction assistant. \
        Summarise the conversation above so a fresh model can pick it up. \
        Preserve: decisions made, file paths touched, errors hit, pending TODOs, \
        and any user-stated constraints. Drop pleasantries and redundant chatter."
        .to_string();
    summary_req.prompt = format!(
        "<conversation>\n{}\n</conversation>\n\n\
         Produce a concise summary in at most {} words. Reply with the summary only.",
        render_for_summary(&older),
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
        text: Some(format!(
            "<compacted_summary>\n{summary}\n</compacted_summary>"
        )),
        images: Vec::new(),
        thinking_content: None,
        timeline: Vec::new(),
    };

    let recent: Vec<HistoryTurn> = chat.history.split_off(split);
    chat.history.clear();
    // Skills first, then summary, then recent verbatim turns.
    chat.history.extend(skill_reminders);
    chat.history.push(meta);
    chat.history.extend(recent);
    drop_response_cache_chain(chat);
    Ok(())
}

/// Detach the request from any server-side conversation chain.
///
/// With `previous_response_id` set, providers that keep the conversation
/// (Volcengine's Session cache, OpenAI's Responses API) rebuild the *entire*
/// prior context on their side and treat the request's own history as an
/// addition to it. Compacting locally while pointing at that chain therefore
/// saves nothing — the pre-compaction turns are still counted, and the summary
/// arrives as extra text on top of them, making the request bigger than before.
fn drop_response_cache_chain(chat: &mut ChatRequest) {
    chat.previous_response_id = None;
}

/// Last-resort shrink after the provider rejected a request for exceeding its
/// context window.
///
/// Returns `false` when nothing could be dropped, in which case the caller
/// must surface the original error rather than retry an identical request.
pub async fn shrink_after_overflow(
    chat: &mut ChatRequest,
    provider: &ProviderEngine,
    policy: &CompactionPolicy,
) -> bool {
    let before = projected_request_tokens(chat);

    // In-turn tool output is the fastest-growing part of a long agent turn and
    // the cheapest to discard: only the round that produced the pending state
    // has to stay.
    if chat.tool_chain.len() > 1 {
        let drop = chat.tool_chain.len() - 1;
        chat.tool_chain.drain(..drop);
    }

    if chat.history.len() > policy.keep_recent {
        if let Err(e) = compact(chat, provider, policy).await {
            eprintln!("[atelier] compaction after context overflow failed: {e}");
            // The summariser could not run (often for the same reason the main
            // call failed), so fall back to dropping the older turns outright.
            let split = user_aligned_split(&chat.history, chat.history.len() - policy.keep_recent);
            chat.history.drain(..split);
            drop_response_cache_chain(chat);
        }
    }

    // The upstream has just told us the request was too big, so a budget that
    // the local estimate thinks is satisfied is not to be trusted. Run the full
    // ladder — it also caps the single oversized result that is the usual
    // reason one round alone overflows.
    enforce_request_budget(chat, policy);
    projected_request_tokens(chat) < before
}

/// Fold the numbers from an upstream context-length rejection into the policy
/// and the request.
///
/// The rejection is the only place a model's true window is ever stated, so a
/// model missing from the catalog learns its window here — and the retry gets a
/// budget to enforce instead of repeating the request that just failed.
///
/// `message_tokens` is the upstream's own count for the messages alone. Where it
/// exceeds the local estimate, the estimator is under-reading this
/// conversation (dense punctuation, a tokenizer that splits CJK differently),
/// so the window is scaled down by the observed ratio to compensate.
pub fn apply_overflow_report(
    chat: &mut ChatRequest,
    policy: &mut CompactionPolicy,
    report: &crate::error::ContextOverflowReport,
) {
    let Some(window) = report.context_window.filter(|w| *w > 0) else {
        return;
    };

    let estimated = estimate_chat_tokens(chat).max(1);
    let effective = match report.message_tokens.filter(|m| *m > estimated) {
        // Deflate the window by however much the estimate under-reads, so the
        // budget computed from it corresponds to real tokens.
        Some(actual) => ((window as f64) * (estimated as f64 / actual as f64)) as i64,
        None => window,
    };

    let effective = effective.max(MIN_DYNAMIC_THRESHOLD);
    chat.context_window = Some(effective);
    *policy = policy.clone().with_context_window(Some(effective));
}

/// Move `split` back to the nearest turn that starts a user exchange.
///
/// Cutting history at an arbitrary index can leave it starting with an
/// assistant turn, which Anthropic rejects outright and which reads to every
/// other provider as a reply to a question it cannot see. Walking backwards
/// (rather than forwards) keeps at least `keep_recent` turns, so the fallback
/// never drops more context than it was asked to.
fn user_aligned_split(history: &[HistoryTurn], split: usize) -> usize {
    let split = split.min(history.len());
    (0..=split)
        .rev()
        .find(|&i| i == history.len() || history[i].role == "user")
        .unwrap_or(split)
}

/// Flatten older turns into a bounded plain-text transcript for the
/// summariser. Each entry is truncated, and the whole block is middle-elided
/// so both the start of the conversation and the most recent turns survive.
fn render_for_summary(turns: &[HistoryTurn]) -> String {
    let mut entries: Vec<String> = Vec::new();
    for turn in turns {
        if let Some(text) = turn
            .text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            entries.push(format!("[{}] {}", turn.role, truncate(text)));
        }
        for seg in &turn.timeline {
            match seg {
                TimelineSegment::AgentStage { .. } => {}
                TimelineSegment::Text { text, .. } => {
                    let text = text.trim();
                    if !text.is_empty() {
                        entries.push(format!("[assistant] {}", truncate(text)));
                    }
                }
                TimelineSegment::ToolRound { calls, results, .. } => {
                    let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
                    let outcome = if results.iter().any(|r| r.is_error) {
                        "error"
                    } else {
                        "ok"
                    };
                    entries.push(format!("[tools] {} -> {outcome}", names.join(", ")));
                }
            }
        }
    }
    elide_middle(entries)
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= SUMMARY_ENTRY_MAX_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(SUMMARY_ENTRY_MAX_CHARS).collect();
    format!("{kept}…(truncated)")
}

/// Keep entries from both ends until the token budget is spent. The opening of
/// a conversation carries the task definition and the tail carries the current
/// state; the middle is what a summary can most afford to lose.
fn elide_middle(entries: Vec<String>) -> String {
    let costs: Vec<i64> = entries.iter().map(|e| estimate_text_tokens(e)).collect();
    if costs.iter().sum::<i64>() <= SUMMARY_INPUT_MAX_TOKENS {
        return entries.join("\n");
    }

    let mut head: Vec<String> = Vec::new();
    let mut tail: Vec<String> = Vec::new();
    let mut budget = SUMMARY_INPUT_MAX_TOKENS;
    let (mut lo, mut hi) = (0usize, entries.len());
    let mut take_tail = true;
    while lo < hi {
        let idx = if take_tail { hi - 1 } else { lo };
        if costs[idx] > budget {
            break;
        }
        budget -= costs[idx];
        if take_tail {
            tail.push(entries[idx].clone());
            hi -= 1;
        } else {
            head.push(entries[idx].clone());
            lo += 1;
        }
        take_tail = !take_tail;
    }
    tail.reverse();

    let mut out = head;
    if lo < hi {
        out.push(format!("…({} earlier entries omitted)…", hi - lo));
    }
    out.extend(tail);
    out.join("\n")
}

fn extract_invoked_skill_turns(turns: &[HistoryTurn]) -> Vec<HistoryTurn> {
    const MARKER: &str = "Continue following the ";
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::<String>::new();
    for turn in turns {
        let Some(text) = turn.text.as_deref() else {
            continue;
        };
        if !text.contains("<system-reminder>") || !text.contains(MARKER) {
            continue;
        }
        // Deduplicate by body so repeated cites don't balloon history.
        if seen.insert(text.to_string()) {
            out.push(turn.clone());
        }
    }
    out
}

#[cfg(test)]
mod threshold_tests {
    use super::*;

    fn policy() -> CompactionPolicy {
        CompactionPolicy::default()
    }

    /// A 32k model must not inherit the 128k-oriented default, or it overflows
    /// long before compaction is ever considered.
    #[test]
    fn small_window_lowers_the_threshold() {
        assert_eq!(
            policy()
                .with_context_window(Some(32_000))
                .effective_threshold(),
            22_400
        );
    }

    #[test]
    fn large_window_keeps_the_fixed_default() {
        assert_eq!(
            policy()
                .with_context_window(Some(1_048_576))
                .effective_threshold(),
            120_000
        );
    }

    #[test]
    fn tiny_window_never_drops_below_the_floor() {
        assert_eq!(
            policy()
                .with_context_window(Some(4_000))
                .effective_threshold(),
            MIN_DYNAMIC_THRESHOLD
        );
    }

    #[test]
    fn unknown_window_keeps_the_fixed_default() {
        assert_eq!(
            policy().with_context_window(None).effective_threshold(),
            120_000
        );
        assert_eq!(
            policy().with_context_window(Some(0)).effective_threshold(),
            120_000
        );
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::ai::chat::{ChatRequest, ProviderConfig};
    use crate::data::settings::ModelParamSettings;

    /// A chat carrying `turns` history entries of `chars_per_turn` ASCII
    /// characters each, on a DeepSeek-sized window.
    fn chat(turns: usize, chars_per_turn: usize, max_tokens: Option<i64>) -> ChatRequest {
        ChatRequest {
            provider: ProviderConfig {
                id: "deepseek".into(),
                name: "DeepSeek".into(),
                sdk: "openai".into(),
                endpoint: "https://api.deepseek.com/v1/chat/completions".into(),
                api_key: "k".into(),
                context_cache_enabled: false,
                safety_threshold: None,
            },
            model: "deepseek-v4-pro".into(),
            prompt: "continue".into(),
            attachments: Vec::new(),
            system_prompt: String::new(),
            history: (0..turns)
                .map(|_| HistoryTurn {
                    role: "user".into(),
                    text: Some("a".repeat(chars_per_turn)),
                    images: Vec::new(),
                    thinking_content: None,
                    timeline: Vec::new(),
                })
                .collect(),
            parameters: crate::ai::parameters::factory().build(
                "auto".into(),
                "auto".into(),
                ModelParamSettings {
                    max_tokens,
                    ..Default::default()
                },
            ),
            tools: Vec::new(),
            tool_chain: Vec::new(),
            tool_results: Vec::new(),
            pending_assistant_turn: None,
            previous_response_id: None,
            context_cache_enabled: false,
            context_window: Some(1_048_576),
            todo_snapshot: None,
            route_providers: Vec::new(),
            forced_tools: Vec::new(),
        }
    }

    /// A chat whose bulk sits in in-turn tool output rather than history — the
    /// shape of a long agent run, and the one that used to escape every check.
    fn chat_with_tool_rounds(
        history_turns: usize,
        rounds: usize,
        tokens_per_round: usize,
        max_tokens: Option<i64>,
    ) -> ChatRequest {
        use crate::ai::chat::{PendingAssistantTurn, ProviderToolCall, ToolResultMessage};

        let mut c = chat(history_turns, 400, max_tokens);
        c.tool_chain = (0..rounds)
            .map(|i| crate::ai::chat::ToolChainRound {
                assistant: PendingAssistantTurn {
                    text: None,
                    thinking_content: None,
                    tool_calls: vec![ProviderToolCall::new(
                        format!("call{i}"),
                        "Read",
                        serde_json::json!({ "path": "manuscript.txt" }),
                    )],
                },
                results: vec![ToolResultMessage {
                    tool_call_id: format!("call{i}"),
                    // 4 ASCII characters per estimated token.
                    content: serde_json::json!({ "text": "m".repeat(tokens_per_round * 4) }),
                    is_error: false,
                }],
            })
            .collect();
        c
    }

    fn policy_for(chat: &ChatRequest) -> CompactionPolicy {
        CompactionPolicy::default().with_context_window(chat.context_window)
    }

    /// The reported failure: messages alone fit, but the completion
    /// reservation pushes the total past the window.
    #[test]
    fn oversized_completion_reservation_is_caught() {
        let c = chat(1, 3_600_000, Some(200_000));
        assert!(exceeds_budget(&c, &policy_for(&c)));
    }

    #[test]
    fn max_tokens_is_clamped_to_the_remaining_room() {
        let mut c = chat(1, 3_600_000, Some(200_000));
        let p = policy_for(&c);
        clamp_completion_budget(&mut c, &p);
        let clamped = c.parameters.model.max_tokens.expect("still set");
        assert!(clamped < 200_000);
        assert!(clamped >= MIN_COMPLETION_TOKENS);
    }

    #[test]
    fn comfortable_requests_keep_the_configured_max_tokens() {
        let mut c = chat(1, 400, Some(8_192));
        let p = policy_for(&c);
        assert!(!exceeds_budget(&c, &p));
        clamp_completion_budget(&mut c, &p);
        assert_eq!(c.parameters.model.max_tokens, Some(8_192));
    }

    /// An unknown window means no enforcement is *possible*, which is why the
    /// resolution layer no longer produces one — see
    /// `llm_catalog::DEFAULT_CONTEXT_WINDOW`. Kept as documentation of what the
    /// policy does when handed `None` directly.
    #[test]
    fn unknown_window_disables_enforcement() {
        let mut c = chat(1, 3_600_000, Some(200_000));
        c.context_window = None;
        let p = CompactionPolicy::default().with_context_window(None);
        assert!(!exceeds_budget(&c, &p));
        clamp_completion_budget(&mut c, &p);
        assert_eq!(c.parameters.model.max_tokens, Some(200_000));
    }

    /// The bug this whole change exists for: on the first call of a turn there
    /// is no reported usage, so an over-budget history rebuilt from the
    /// database used to sail straight into a 400.
    #[test]
    fn first_call_of_a_turn_compacts_on_the_estimate_alone() {
        let c = chat(9, 100_000, Some(200_000));
        assert!(should_summarise_history(
            &c,
            &TokenUsage::default(),
            &policy_for(&c)
        ));
    }

    /// A multi-round turn sums each round's prompt into `total_tokens`, which
    /// would trip the threshold while the real context is still small.
    #[test]
    fn summed_rounds_do_not_trigger_compaction() {
        let c = chat(20, 400, None);
        let usage = TokenUsage {
            prompt_tokens: Some(130_000),
            total_tokens: Some(131_000),
            last_prompt_tokens: Some(33_000),
            ..Default::default()
        };
        assert!(!should_summarise_history(&c, &usage, &policy_for(&c)));
    }

    #[test]
    fn reported_occupancy_triggers_compaction() {
        let c = chat(20, 400, None);
        let usage = TokenUsage {
            prompt_tokens: Some(400_000),
            total_tokens: Some(401_000),
            last_prompt_tokens: Some(125_000),
            ..Default::default()
        };
        assert!(should_summarise_history(&c, &usage, &policy_for(&c)));
    }

    #[test]
    fn falls_back_to_total_when_provider_omits_last_prompt() {
        let c = chat(20, 400, None);
        let usage = TokenUsage {
            total_tokens: Some(125_000),
            ..Default::default()
        };
        assert!(should_summarise_history(&c, &usage, &policy_for(&c)));
    }

    /// Summarising needs something to summarise. Budget enforcement, however,
    /// must still run: the early return used to short-circuit *both*, which is
    /// how a short-history run with a huge tool chain reached the provider
    /// completely unpoliced.
    #[test]
    fn short_history_is_not_summarised_but_is_still_enforced() {
        let mut c = chat_with_tool_rounds(2, 5, 200_000, Some(128_000));
        let p = policy_for(&c);
        let usage = TokenUsage {
            last_prompt_tokens: Some(500_000),
            ..Default::default()
        };

        assert!(
            !should_summarise_history(&c, &usage, &p),
            "there is no history worth summarising"
        );
        assert!(
            exceeds_budget(&c, &p),
            "precondition: the request is over the window"
        );

        assert!(enforce_request_budget(&mut c, &p));
        assert!(!exceeds_budget(&c, &p));
    }

    /// The reported failure end to end, on the conservative default window a
    /// model missing from the catalog now gets: 940k of tool output plus a
    /// 128000-token completion reservation must not leave the process.
    #[test]
    fn a_default_window_still_enforces_a_budget() {
        let mut c = chat_with_tool_rounds(2, 5, 200_000, Some(128_000));
        c.context_window = Some(crate::data::llm_catalog::DEFAULT_CONTEXT_WINDOW);
        let p = policy_for(&c);

        assert!(enforce_request_budget(&mut c, &p));

        let budget = p.request_budget().expect("window is known");
        assert!(projected_request_tokens(&c) <= budget);
        let max_tokens = c.parameters.model.max_tokens.expect("still set");
        assert!(
            max_tokens < 128_000,
            "the completion reservation must shrink, got {max_tokens}"
        );
        assert!(max_tokens >= MIN_COMPLETION_TOKENS);
    }

    /// Shrinking the prompt has to hand the freed room back to the completion,
    /// not leave the reply capped at its floor for the rest of the run.
    #[test]
    fn freed_room_is_returned_to_the_completion() {
        let mut c = chat_with_tool_rounds(2, 5, 200_000, Some(16_384));
        let p = policy_for(&c);
        assert!(enforce_request_budget(&mut c, &p));
        assert_eq!(
            c.parameters.model.max_tokens,
            Some(16_384),
            "the whole configured reservation fits once the chain is windowed"
        );
    }

    /// Requests that were never over budget must come out byte-identical, or
    /// every turn would invalidate the provider's prompt cache.
    #[test]
    fn comfortable_requests_are_left_alone_by_enforcement() {
        let mut c = chat_with_tool_rounds(2, 3, 500, Some(8_192));
        let p = policy_for(&c);
        let before = c.clone();
        assert!(enforce_request_budget(&mut c, &p));
        assert_eq!(c.tool_chain.len(), before.tool_chain.len());
        assert_eq!(c.history.len(), before.history.len());
        assert_eq!(c.parameters.model.max_tokens, Some(8_192));
        assert_eq!(
            estimate_chat_tokens(&c),
            estimate_chat_tokens(&before),
            "nothing was elided"
        );
    }
}

#[cfg(test)]
mod split_alignment_tests {
    use super::*;

    fn history(roles: &[&str]) -> Vec<HistoryTurn> {
        roles
            .iter()
            .map(|role| HistoryTurn {
                role: (*role).into(),
                text: Some("x".into()),
                images: Vec::new(),
                thinking_content: None,
                timeline: Vec::new(),
            })
            .collect()
    }

    /// Cutting mid-exchange would leave the history starting with an assistant
    /// reply, which Anthropic rejects outright.
    #[test]
    fn a_split_landing_on_an_assistant_turn_moves_back() {
        let h = history(&["user", "assistant", "user", "assistant"]);
        assert_eq!(user_aligned_split(&h, 3), 2);
    }

    #[test]
    fn a_split_already_on_a_user_turn_is_kept() {
        let h = history(&["user", "assistant", "user", "assistant"]);
        assert_eq!(user_aligned_split(&h, 2), 2);
    }

    /// Backwards alignment may keep more than `keep_recent`, never less.
    #[test]
    fn alignment_never_drops_more_than_requested() {
        let h = history(&["user", "assistant", "assistant", "assistant"]);
        assert!(user_aligned_split(&h, 3) <= 3);
    }

    #[test]
    fn a_history_without_user_turns_falls_back_to_the_raw_split() {
        let h = history(&["assistant", "assistant", "assistant"]);
        assert_eq!(user_aligned_split(&h, 2), 2);
    }
}

#[cfg(test)]
mod summary_input_tests {
    use super::*;

    fn turn(role: &str, text: &str) -> HistoryTurn {
        HistoryTurn {
            role: role.into(),
            text: Some(text.into()),
            images: Vec::new(),
            thinking_content: None,
            timeline: Vec::new(),
        }
    }

    /// The summariser call must fit even when the history it summarises does
    /// not — otherwise compaction fails for the same reason it was needed.
    #[test]
    fn oversized_history_is_bounded_before_summarising() {
        let turns: Vec<HistoryTurn> = (0..200)
            .map(|i| turn("user", &format!("{i} {}", "y".repeat(3_000))))
            .collect();
        let rendered = render_for_summary(&turns);
        assert!(estimate_text_tokens(&rendered) <= SUMMARY_INPUT_MAX_TOKENS);
        assert!(rendered.contains("earlier entries omitted"));
    }

    #[test]
    fn both_ends_of_the_conversation_survive() {
        let turns: Vec<HistoryTurn> = (0..200)
            .map(|i| turn("user", &format!("marker{i} {}", "y".repeat(3_000))))
            .collect();
        let rendered = render_for_summary(&turns);
        assert!(rendered.contains("marker0"));
        assert!(rendered.contains("marker199"));
    }

    #[test]
    fn small_history_is_rendered_whole() {
        let rendered = render_for_summary(&[turn("user", "hello"), turn("assistant", "hi")]);
        assert_eq!(rendered, "[user] hello\n[assistant] hi");
    }
}
