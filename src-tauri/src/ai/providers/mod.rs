use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod claude;
mod gemini;
mod grok;
mod openai;

use crate::ai::chat::{ChatRequest, GenerateResponse};
use crate::error::{AppError, AppResult};

pub const OPENAI_SDK: &str = "openai";
pub const OPENAI_RESPONSES_SDK: &str = "openai-responses";
pub const GEMINI_SDK: &str = "gemini";
pub const CLAUDE_SDK: &str = "claude";
pub const GROK_SDK: &str = "grok";
pub const SUPPORTED_SDKS: &[&str] = &[
    OPENAI_SDK,
    OPENAI_RESPONSES_SDK,
    GEMINI_SDK,
    CLAUDE_SDK,
    GROK_SDK,
];

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = AppResult<GenerateResponse>> + Send + 'a>>;

pub trait ChatProvider: Send + Sync {
    fn sdk(&self) -> &'static str;
    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a>;
}

#[derive(Clone)]
pub struct ProviderFactory {
    providers: HashMap<&'static str, Arc<dyn ChatProvider>>,
}

impl ProviderFactory {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register<P>(mut self, provider: P) -> Self
    where
        P: ChatProvider + 'static,
    {
        self.providers.insert(provider.sdk(), Arc::new(provider));
        self
    }

    pub async fn chat(&self, request: ChatRequest) -> AppResult<GenerateResponse> {
        let sdk = normalize_sdk(&request.provider.sdk);
        let provider = self.providers.get(sdk.as_str()).ok_or_else(|| {
            AppError::Config(format!(
                "unsupported provider sdk: {}",
                request.provider.sdk
            ))
        })?;
        provider.chat(request).await
    }
}

impl Default for ProviderFactory {
    fn default() -> Self {
        Self::new()
            .register(openai::OpenAiProvider::new())
            .register(openai::OpenAiResponsesProvider::new())
            .register(gemini::GeminiProvider::new())
            .register(claude::ClaudeProvider::new())
            .register(grok::GrokProvider::new())
    }
}

pub fn normalize_sdk(sdk: &str) -> String {
    let sdk = sdk.trim().to_ascii_lowercase();
    if sdk.is_empty() {
        OPENAI_SDK.to_string()
    } else if sdk == "openrouter" || sdk == "deepseek" {
        OPENAI_SDK.to_string()
    } else {
        sdk
    }
}

pub fn is_supported_sdk(sdk: &str) -> bool {
    let sdk = normalize_sdk(sdk);
    SUPPORTED_SDKS.contains(&sdk.as_str())
}
