use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod ark_images;
mod ark_video;
mod claude;
mod gemini;
mod grok;
pub mod model_list;
mod openai;

use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::error::{AppError, AppResult};

/// Time we wait for the TCP/TLS connection to be established. Connecting is
/// bounded regardless of how long the model then thinks.
const CONNECT_TIMEOUT_SECS: u64 = 60;

/// Idle time before the OS starts sending TCP keepalive probes on an
/// otherwise-silent connection. This is how we detect a dead server without
/// imposing any limit on legitimate work.
const TCP_KEEPALIVE_SECS: u64 = 30;

/// Shared HTTP client for chat/text providers (openai, claude, gemini, ...).
///
/// Deliberately sets **no total timeout and no read (idle) timeout**: as long
/// as the connection to the upstream is alive, generation may run forever, so
/// even models that think for a very long time are never cut off locally.
///
/// Liveness is enforced purely at the transport layer via TCP keepalive: if
/// the server actually drops the connection, keepalive probes fail and the
/// socket errors out — which is exactly "we only watch whether the server is
/// still connected". Only connection establishment is bounded
/// ([`CONNECT_TIMEOUT_SECS`]).
pub(crate) fn build_chat_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .tcp_keepalive(std::time::Duration::from_secs(TCP_KEEPALIVE_SECS))
        .build()
}

pub const OPENAI_SDK: &str = "openai";
pub const OPENAI_RESPONSES_SDK: &str = "openai-responses";
pub const GEMINI_SDK: &str = "gemini";
pub const CLAUDE_SDK: &str = "claude";
pub const GROK_SDK: &str = "grok";
pub const ARK_IMAGES_SDK: &str = "ark-images";
pub const ARK_VIDEO_SDK: &str = "ark-video";
pub const SUPPORTED_SDKS: &[&str] = &[
    OPENAI_SDK,
    OPENAI_RESPONSES_SDK,
    GEMINI_SDK,
    CLAUDE_SDK,
    GROK_SDK,
    ARK_IMAGES_SDK,
    ARK_VIDEO_SDK,
];

pub type ProviderFuture<'a> =
    Pin<Box<dyn Future<Output = AppResult<GenerateResponse>> + Send + 'a>>;

pub trait ChatProvider: Send + Sync {
    fn sdk(&self) -> &'static str;
    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a>;

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        _on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        self.chat(request)
    }
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

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> AppResult<GenerateResponse> {
        let sdk = normalize_sdk(&request.provider.sdk);
        let provider = self.providers.get(sdk.as_str()).ok_or_else(|| {
            AppError::Config(format!(
                "unsupported provider sdk: {}",
                request.provider.sdk
            ))
        })?;
        provider.chat_stream(request, on_text_delta).await
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
            .register(ark_images::ArkImagesProvider::new())
            .register(ark_video::ArkVideoProvider::new())
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
