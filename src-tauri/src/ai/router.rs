//! Helpers for assembling a [`ChatRequest`] from persisted settings.
//!
//! Historically this module also exposed `chat()` / `chat_stream()`
//! wrappers around [`crate::ai::providers::ProviderFactory`]. Those were
//! removed in favour of the agent-layer entry point
//! [`crate::ai::agent::run_agent`] with [`crate::ai::agent::ProviderQueryEngine`],
//! which adds task tracking, tool turns, and cancellation on top of the same
//! provider call.

use crate::ai::chat::{AttachmentBytes, ChatRequest, HistoryTurn, ProviderConfig};
use crate::ai::parameters::GenerationParameters;
use crate::ai::providers;
use crate::data::settings::ModelProvider;
use crate::error::{AppError, AppResult};

/// Assemble a [`ChatRequest`] from an already-resolved provider + model.
///
/// The provider and model are resolved per-session upstream (see
/// `resolve_session_generation`), so this function no longer reads the global
/// settings for model identity.
pub fn build_chat_request(
    provider: &ModelProvider,
    model: &str,
    prompt: String,
    attachments: Vec<AttachmentBytes>,
    system_prompt: String,
    history: Vec<HistoryTurn>,
    parameters: GenerationParameters,
) -> AppResult<ChatRequest> {
    if provider.api_key.trim().is_empty() {
        return Err(AppError::Config("missing provider API key".into()));
    }
    if provider.endpoint.trim().is_empty() {
        return Err(AppError::Config("missing provider endpoint".into()));
    }
    if model.trim().is_empty() {
        return Err(AppError::Config("missing active model".into()));
    }

    Ok(ChatRequest {
        provider: ProviderConfig {
            id: provider.id.clone(),
            name: provider.name.clone(),
            sdk: providers::normalize_sdk(&provider.sdk),
            endpoint: provider.endpoint.clone(),
            api_key: provider.api_key.clone(),
        },
        model: model.to_string(),
        prompt,
        attachments,
        system_prompt,
        history,
        parameters,
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
    })
}
