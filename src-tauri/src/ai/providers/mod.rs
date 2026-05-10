use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

mod openrouter;

use crate::ai::chat::{ChatRequest, GenerateResponse};
use crate::error::{AppError, AppResult};

pub const OPENROUTER_SDK: &str = "openrouter";

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
        Self::new().register(openrouter::OpenRouterProvider::new())
    }
}

pub fn normalize_sdk(sdk: &str) -> String {
    let sdk = sdk.trim();
    if sdk.is_empty() {
        OPENROUTER_SDK.to_string()
    } else {
        sdk.to_ascii_lowercase()
    }
}
