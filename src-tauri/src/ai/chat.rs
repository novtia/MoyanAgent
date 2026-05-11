use std::sync::Arc;

use crate::ai::parameters::GenerationParameters;
use crate::ai::tokens::TokenUsage;

pub type TextDeltaCallback = Arc<dyn Fn(String) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct ImageResult {
    pub bytes: Vec<u8>,
    pub mime: String,
}

#[derive(Debug, Clone)]
pub struct GenerateResponse {
    pub images: Vec<ImageResult>,
    pub text: Option<String>,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone)]
pub struct AttachmentBytes {
    pub bytes: Vec<u8>,
    pub mime: String,
}

#[derive(Debug, Clone)]
pub struct HistoryTurn {
    pub role: String,
    pub text: Option<String>,
    pub images: Vec<AttachmentBytes>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub sdk: String,
    pub endpoint: String,
    pub api_key: String,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub provider: ProviderConfig,
    pub model: String,
    pub prompt: String,
    pub attachments: Vec<AttachmentBytes>,
    pub system_prompt: String,
    pub history: Vec<HistoryTurn>,
    pub parameters: GenerationParameters,
}
