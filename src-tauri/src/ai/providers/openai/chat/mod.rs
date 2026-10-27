pub(crate) mod body;
pub(crate) mod parse;
pub(crate) mod stream;

use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::error::{AppError, AppResult};

use super::common::{post_with_retries, provider_label, set_streaming};
use super::openrouter::{
    apply_openrouter_gemini_arg_streaming, is_openrouter_endpoint, post_openrouter_chat,
    post_openrouter_chat_stream,
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

    let client = chat_http_client(&request.provider.endpoint)?;

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
    apply_openrouter_gemini_arg_streaming(&mut body, &request);
    let provider_label = provider_label(&request);
    let openrouter_compat = is_openrouter_endpoint(&request.provider.endpoint);

    let client = chat_http_client(&request.provider.endpoint)?;

    if openrouter_compat {
        post_openrouter_chat_stream(&client, &request, &mut body, &provider_label, on_text_delta)
            .await
    } else {
        post_stream_with_retries(&client, &request, &body, &provider_label, on_text_delta).await
    }
}

/// Loopback llama-server must not inherit the app HTTP proxy, or the rewrite
/// call is sent to the proxy and never reaches 127.0.0.1.
fn chat_http_client(endpoint: &str) -> reqwest::Result<reqwest::Client> {
    if !is_loopback_endpoint(endpoint) {
        return crate::ai::providers::build_chat_client();
    }
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(
            crate::ai::providers::CONNECT_TIMEOUT_SECS,
        ))
        .tcp_keepalive(std::time::Duration::from_secs(
            crate::ai::providers::TCP_KEEPALIVE_SECS,
        ))
        .no_proxy()
        .build()
}

fn is_loopback_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    match url.host_str() {
        Some("localhost" | "127.0.0.1" | "::1") => true,
        Some(host) if host.starts_with("127.") => true,
        _ => false,
    }
}
