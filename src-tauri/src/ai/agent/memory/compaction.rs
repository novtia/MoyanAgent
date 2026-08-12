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
use crate::ai::chat::{ChatRequest, HistoryTurn, TimelineSegment};
use crate::ai::tokens::{estimate_chat_tokens, estimate_text_tokens, TokenUsage};
use crate::error::AppResult;

/// Fraction of the context window that may be occupied by the prompt before
/// compaction kicks in.
const DEFAULT_OCCUPANCY_RATIO: f64 = 0.7;

/// Never compact below this, however small the advertised window: a threshold
/// of a few hundred tokens would summarise on every single turn.
const MIN_DYNAMIC_THRESHOLD: i64 = 8_000;

/// Slack kept between the projected request size and the hard window limit.
/// Absorbs the estimator's error (it approximates a real tokenizer) plus
/// provider-side framing we cannot see.
const BUDGET_HEADROOM_RATIO: f64 = 0.1;

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
pub fn should_compact_chat(
    chat: &ChatRequest,
    usage: &TokenUsage,
    policy: &CompactionPolicy,
) -> bool {
    if chat.history.len() <= policy.keep_recent + 1 {
        return false;
    }
    let reported = usage
        .last_prompt_tokens
        .or(usage.total_tokens)
        .unwrap_or(0);
    let occupancy = reported.max(estimate_chat_tokens(chat));
    occupancy >= policy.effective_threshold() || exceeds_budget(chat, policy)
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
        text: Some(format!("<compacted_summary>\n{summary}\n</compacted_summary>")),
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
    Ok(())
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
    let before = estimate_chat_tokens(chat);

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
            let split = chat.history.len() - policy.keep_recent;
            chat.history.drain(..split);
        }
    }

    clamp_completion_budget(chat, policy);
    estimate_chat_tokens(chat) < before
}

/// Flatten older turns into a bounded plain-text transcript for the
/// summariser. Each entry is truncated, and the whole block is middle-elided
/// so both the start of the conversation and the most recent turns survive.
fn render_for_summary(turns: &[HistoryTurn]) -> String {
    let mut entries: Vec<String> = Vec::new();
    for turn in turns {
        if let Some(text) = turn.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
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
            policy().with_context_window(Some(32_000)).effective_threshold(),
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
            policy().with_context_window(Some(4_000)).effective_threshold(),
            MIN_DYNAMIC_THRESHOLD
        );
    }

    #[test]
    fn unknown_window_keeps_the_fixed_default() {
        assert_eq!(policy().with_context_window(None).effective_threshold(), 120_000);
        assert_eq!(policy().with_context_window(Some(0)).effective_threshold(), 120_000);
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
        }
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
        assert!(should_compact_chat(&c, &TokenUsage::default(), &policy_for(&c)));
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
        assert!(!should_compact_chat(&c, &usage, &policy_for(&c)));
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
        assert!(should_compact_chat(&c, &usage, &policy_for(&c)));
    }

    #[test]
    fn falls_back_to_total_when_provider_omits_last_prompt() {
        let c = chat(20, 400, None);
        let usage = TokenUsage {
            total_tokens: Some(125_000),
            ..Default::default()
        };
        assert!(should_compact_chat(&c, &usage, &policy_for(&c)));
    }

    /// Compacting needs something to compact; below that the only lever left
    /// is the `max_tokens` clamp.
    #[test]
    fn short_history_is_never_compacted() {
        let c = chat(CompactionPolicy::default().keep_recent + 1, 400, None);
        let usage = TokenUsage {
            last_prompt_tokens: Some(500_000),
            ..Default::default()
        };
        assert!(!should_compact_chat(&c, &usage, &policy_for(&c)));
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
