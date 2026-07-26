pub(crate) mod body;
pub(crate) mod parse;
pub(crate) mod stream;

use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::error::{AppError, AppResult};

use super::common::{post_with_retries, provider_label, set_streaming};
use super::openrouter::{
    is_openrouter_endpoint, post_openrouter_chat, post_openrouter_chat_stream,
};
use body::build_chat_body;
use parse::parse_openai_like_response;
use stream::post_stream_with_retries;

pub(crate) async fn generate_chat(
    request: ChatRequest,
    allow_image_parts: bool,
) -> AppResult<GenerateResponse> {
    if !allow_image_parts && !request.attachments.is_empty() {
        return Err(AppError::Config(
            "the selected provider sdk does not support image attachments".into(),
        ));
    }

    let mut body = build_chat_body(&request, allow_image_parts);
    let provider_label = provider_label(&request);
    let openrouter_compat = is_openrouter_endpoint(&request.provider.endpoint);

    let client = crate::ai::providers::build_chat_client()?;

    let final_txt = if openrouter_compat {
        post_openrouter_chat(&client, &request, &mut body, &provider_label).await?
    } else {
        post_with_retries(&client, &request, &body, &provider_label).await?
    };

    parse_openai_like_response(&final_txt)
}

pub(crate) async fn generate_chat_stream(
    request: ChatRequest,
    allow_image_parts: bool,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    if !allow_image_parts && !request.attachments.is_empty() {
        return Err(AppError::Config(
            "the selected provider sdk does not support image attachments".into(),
        ));
    }

    let mut body = build_chat_body(&request, allow_image_parts);
    set_streaming(&mut body, true);
    let provider_label = provider_label(&request);
    let openrouter_compat = is_openrouter_endpoint(&request.provider.endpoint);

    let client = crate::ai::providers::build_chat_client()?;

    if openrouter_compat {
        post_openrouter_chat_stream(&client, &request, &mut body, &provider_label, on_text_delta)
            .await
    } else {
        post_stream_with_retries(&client, &request, &body, &provider_label, on_text_delta).await
    }
}
