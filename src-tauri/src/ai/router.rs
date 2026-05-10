use crate::ai::chat::{
    AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ProviderConfig,
};
use crate::ai::parameters::GenerationParameters;
use crate::ai::providers;
use crate::data::settings;
use crate::error::{AppError, AppResult};

pub fn build_chat_request(
    settings: &settings::Settings,
    prompt: String,
    attachments: Vec<AttachmentBytes>,
    system_prompt: String,
    history: Vec<HistoryTurn>,
    parameters: GenerationParameters,
) -> AppResult<ChatRequest> {
    let provider = settings::active_provider(settings)
        .ok_or_else(|| AppError::Config("no enabled model provider configured".into()))?;

    if provider.api_key.trim().is_empty() {
        return Err(AppError::Config("missing provider API key".into()));
    }
    if provider.endpoint.trim().is_empty() {
        return Err(AppError::Config("missing provider endpoint".into()));
    }
    if settings.model.trim().is_empty() {
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
        model: settings.model.clone(),
        prompt,
        attachments,
        system_prompt,
        history,
        parameters,
    })
}

pub async fn chat(request: ChatRequest) -> AppResult<GenerateResponse> {
    providers::ProviderFactory::default().chat(request).await
}
