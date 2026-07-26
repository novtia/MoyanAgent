use futures_util::StreamExt;
use serde_json::Value;

use crate::ai::chat::{
    emit_thinking_deltas, ChatRequest, GenerateResponse, ImageResult, StreamDelta,
    TextDeltaCallback,
};
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::{AppError, AppResult};

use super::super::common::{
    collect_response_images, debug_log_sse_event, debug_log_upstream_request,
    debug_log_upstream_response_text, emit_final_text_if_needed, finalize_stream_response,
    find_sse_event_end, is_empty_stream_upstream_error, is_json_response, is_retryable_status,
    merge_tool_call_deltas, merge_usage, post_with_retries, should_retry_transport,
    sleep_for_attempt, sse_data_payload, stream_read_error, top_level_error_message,
    upstream_debug, upstream_error_message, upstream_rejects_streaming, without_streaming,
    PendingStreamToolCall, MAX_ATTEMPTS,
};
use super::parse::parse_openai_like_response;

pub(crate) async fn post_stream_with_retries(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    for attempt in 1..=MAX_ATTEMPTS {
        if attempt == 1 {
            debug_log_upstream_request(provider_label, &request.provider.endpoint, body);
        }
        let resp = client
            .post(&request.provider.endpoint)
            .bearer_auth(&request.provider.api_key)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(err) => {
                if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                    sleep_for_attempt(attempt).await;
                    continue;
                }
                return Err(err.into());
            }
        };

        let status = resp.status();
        if status.is_success() {
            return match parse_openai_chat_success(resp, on_text_delta.clone()).await {
                Ok(r) => Ok(r),
                Err(e) if is_empty_stream_upstream_error(&e) => {
                    fallback_openai_chat_response(
                        client,
                        request,
                        body,
                        provider_label,
                        on_text_delta,
                    )
                    .await
                }
                Err(e) => Err(e),
            };
        }

        let txt = resp.text().await?;
        let msg = upstream_error_message(&txt);
        if attempt < MAX_ATTEMPTS && is_retryable_status(status) {
            sleep_for_attempt(attempt).await;
            continue;
        }
        if upstream_rejects_streaming(status, &msg) {
            return fallback_openai_chat_response(
                client,
                request,
                body,
                provider_label,
                on_text_delta,
            )
            .await;
        }
        return Err(AppError::Upstream(format!(
            "{} HTTP {}: {}",
            provider_label, status, msg
        )));
    }
    unreachable!("HTTP attempts should return or branch before completing the loop");
}
pub(crate) async fn parse_openai_chat_success(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    if is_json_response(&resp) {
        let final_txt = resp.text().await?;
        if upstream_debug() {
            debug_log_upstream_response_text("openai chat success (JSON, not SSE)", &final_txt);
        }
        let parsed = parse_openai_like_response(&final_txt)?;
        emit_final_text_if_needed(&parsed, &on_text_delta);
        return Ok(parsed);
    }
    consume_openai_chat_stream(resp, on_text_delta).await
}
pub(crate) async fn consume_openai_chat_stream(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut images = Vec::new();
    let mut usage = TokenUsage::default();
    let mut tool_calls: Vec<PendingStreamToolCall> = Vec::new();
    let mut sse_debug_emitted = 0u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(stream_read_error)?;
        buffer.extend_from_slice(&chunk);
        while let Some((event_end, sep_len)) = find_sse_event_end(&buffer) {
            let drained: Vec<u8> = buffer.drain(..event_end + sep_len).collect();
            let event = String::from_utf8_lossy(&drained[..event_end]);
            debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
            if handle_openai_chat_sse_event(
                &event,
                &mut text,
                &mut thinking,
                &mut images,
                &mut usage,
                &mut tool_calls,
                &on_text_delta,
            )? {
                return finalize_stream_response(text, thinking, images, usage, tool_calls);
            }
        }
    }

    if !buffer.is_empty() {
        let event = String::from_utf8_lossy(&buffer);
        debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
        handle_openai_chat_sse_event(
            &event,
            &mut text,
            &mut thinking,
            &mut images,
            &mut usage,
            &mut tool_calls,
            &on_text_delta,
        )?;
    }

    finalize_stream_response(text, thinking, images, usage, tool_calls)
}
pub(crate) async fn fallback_openai_chat_response(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let body = without_streaming(body);
    let final_txt = post_with_retries(client, request, &body, provider_label).await?;
    let resp = parse_openai_like_response(&final_txt)?;
    emit_final_text_if_needed(&resp, &on_text_delta);
    Ok(resp)
}
pub(crate) fn handle_openai_chat_sse_event(
    event: &str,
    text: &mut String,
    thinking: &mut String,
    images: &mut Vec<ImageResult>,
    usage: &mut TokenUsage,
    tool_calls: &mut Vec<PendingStreamToolCall>,
    on_text_delta: &TextDeltaCallback,
) -> AppResult<bool> {
    let Some(data) = sse_data_payload(event) else {
        return Ok(false);
    };
    let data = data.trim();
    if data.is_empty() {
        return Ok(false);
    }
    if data == "[DONE]" {
        return Ok(true);
    }

    let v: Value = serde_json::from_str(data).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream SSE event: {err}; data={data}"
        ))
    })?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let (delta, mut new_images, think_delta) = extract_openai_chat_stream_update(&v);
    if !think_delta.is_empty() {
        thinking.push_str(&think_delta);
        emit_thinking_deltas(on_text_delta, &think_delta);
    }
    if !delta.is_empty() {
        text.push_str(&delta);
        (on_text_delta)(StreamDelta::text(delta));
    }
    images.append(&mut new_images);
    merge_tool_call_deltas(&v, tool_calls, on_text_delta);
    merge_usage(usage, tokens::extract_usage(&v));
    Ok(false)
}
pub(crate) fn extract_openai_chat_stream_update(v: &Value) -> (String, Vec<ImageResult>, String) {
    let mut text_parts = Vec::new();
    let mut thinking_parts = Vec::new();
    let mut images = Vec::new();

    if let Some(choices) = v.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                collect_openai_delta_reasoning(delta, &mut thinking_parts);
                collect_openai_delta_text(delta, &mut text_parts);
                collect_response_images(delta, &mut images);
            }
            if let Some(message) = choice.get("message") {
                collect_openai_delta_reasoning(message, &mut thinking_parts);
                collect_openai_delta_text(message, &mut text_parts);
                collect_response_images(message, &mut images);
            }
        }
    }

    (text_parts.concat(), images, thinking_parts.concat())
}

pub(crate) fn collect_openai_delta_reasoning(v: &Value, out: &mut Vec<String>) {
    if let Some(s) = v.get("reasoning").and_then(Value::as_str) {
        if !s.is_empty() {
            out.push(s.to_string());
        }
    }
    // DeepSeek Chat Completions stream: reasoning tokens in `delta.reasoning_content`.
    if let Some(s) = v.get("reasoning_content").and_then(Value::as_str) {
        if !s.is_empty() {
            out.push(s.to_string());
        }
    }
    if let Some(content) = v.get("content") {
        if let Value::Array(parts) = content {
            for part in parts {
                let typ = part.get("type").and_then(Value::as_str).unwrap_or("");
                if typ == "reasoning" {
                    if let Some(s) = part
                        .get("text")
                        .or_else(|| part.get("reasoning"))
                        .and_then(Value::as_str)
                    {
                        if !s.is_empty() {
                            out.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn collect_openai_delta_text(v: &Value, out: &mut Vec<String>) {
    if let Some(content) = v.get("content") {
        match content {
            Value::String(s) => out.push(s.clone()),
            Value::Array(parts) => {
                for part in parts {
                    collect_content_part_text(part, out);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn collect_content_part_text(v: &Value, out: &mut Vec<String>) {
    if let Some(s) = v.get("text").and_then(Value::as_str) {
        out.push(s.to_string());
        return;
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        out.push(s.to_string());
    }
}
