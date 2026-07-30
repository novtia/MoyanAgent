pub(crate) mod body;
pub(crate) mod cache;
pub(crate) mod parse;
pub(crate) mod stream;

use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::error::AppResult;

use super::common::{post_with_retries, provider_label, set_streaming};
use body::build_responses_body;
use cache::{should_disable_responses_cache, should_reset_responses_cache};
use parse::parse_responses_response;
use stream::post_responses_stream_with_retries;

fn disable_request_cache(request: &mut ChatRequest) {
    request.context_cache_enabled = false;
    request.provider.context_cache_enabled = false;
    request.previous_response_id = None;
}

pub(crate) async fn generate_responses(request: ChatRequest) -> AppResult<GenerateResponse> {
    let provider_label = provider_label(&request);
    let client = crate::ai::providers::build_chat_client()?;
    let mut request = request;
    let body = build_responses_body(&request);
    match post_with_retries(&client, &request, &body, &provider_label).await {
        Ok(final_txt) => parse_responses_response(&final_txt),
        Err(err) if should_disable_responses_cache(&err, &request) => {
            disable_request_cache(&mut request);
            let body = build_responses_body(&request);
            let final_txt = post_with_retries(&client, &request, &body, &provider_label).await?;
            parse_responses_response(&final_txt)
        }
        Err(err) if should_reset_responses_cache(&err, &request) => {
            request.previous_response_id = None;
            let body = build_responses_body(&request);
            let final_txt = post_with_retries(&client, &request, &body, &provider_label).await?;
            parse_responses_response(&final_txt)
        }
        Err(err) => Err(err),
    }
}

pub(crate) async fn generate_responses_stream(
    request: ChatRequest,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let provider_label = provider_label(&request);
    let client = crate::ai::providers::build_chat_client()?;
    let mut request = request;
    let mut body = build_responses_body(&request);
    // Responses API reports usage on `response.completed`; do not send
    // chat-completions-only `stream_options`. Ark rejects unknown fields and
    // the error text often contains "stream", which would falsely trigger the
    // non-streaming fallback (`upstream_rejects_streaming`).
    set_streaming(&mut body, false);
    match post_responses_stream_with_retries(
        &client,
        &request,
        &body,
        &provider_label,
        on_text_delta.clone(),
    )
    .await
    {
        Ok(resp) => Ok(resp),
        Err(err) if should_disable_responses_cache(&err, &request) => {
            disable_request_cache(&mut request);
            let mut body = build_responses_body(&request);
            set_streaming(&mut body, false);
            post_responses_stream_with_retries(
                &client,
                &request,
                &body,
                &provider_label,
                on_text_delta,
            )
            .await
        }
        Err(err) if should_reset_responses_cache(&err, &request) => {
            request.previous_response_id = None;
            let mut body = build_responses_body(&request);
            set_streaming(&mut body, false);
            post_responses_stream_with_retries(
                &client,
                &request,
                &body,
                &provider_label,
                on_text_delta,
            )
            .await
        }
        Err(err) => Err(err),
    }
}
