use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::ai::providers::{ChatProvider, ProviderFuture, OPENAI_RESPONSES_SDK, OPENAI_SDK};

use super::chat::{generate_chat, generate_chat_stream};
use super::responses::{generate_responses, generate_responses_stream};

pub struct OpenAiProvider;

impl OpenAiProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for OpenAiProvider {
    fn sdk(&self) -> &'static str {
        OPENAI_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate_chat(request, true).await })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move { generate_chat_stream(request, true, on_text_delta).await })
    }
}

pub struct OpenAiResponsesProvider;

impl OpenAiResponsesProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for OpenAiResponsesProvider {
    fn sdk(&self) -> &'static str {
        OPENAI_RESPONSES_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate_responses(request).await })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move { generate_responses_stream(request, on_text_delta).await })
    }
}
