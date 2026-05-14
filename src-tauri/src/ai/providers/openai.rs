use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use crate::ai::chat::{
    emit_thinking_deltas, AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ImageResult,
    StreamDelta, TextDeltaCallback,
};
use crate::ai::providers::{ChatProvider, ProviderFuture, OPENAI_RESPONSES_SDK, OPENAI_SDK};
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
const MAX_ATTEMPTS: usize = 3;

/// When `ATELIER_DEBUG_UPSTREAM` is `1` / `true` / `yes`, eprintln request JSON
/// bodies and response bodies (truncated for large payloads) for OpenAI-compatible calls.
fn upstream_debug() -> bool {
    matches!(
        std::env::var("ATELIER_DEBUG_UPSTREAM").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn debug_log_upstream_request(label: &str, endpoint: &str, body: &Value) {
    if !upstream_debug() {
        return;
    }
    let body_str = serde_json::to_string_pretty(body)
        .unwrap_or_else(|_| body.to_string());
    eprintln!(
        "[ATELIER_DEBUG_UPSTREAM] {} POST {}\n{}",
        label, endpoint, body_str
    );
}

fn debug_log_upstream_response_text(label: &str, txt: &str) {
    if !upstream_debug() {
        return;
    }
    const MAX: usize = 16_384;
    if txt.len() <= MAX {
        eprintln!("[ATELIER_DEBUG_UPSTREAM] {} response JSON:\n{}", label, txt);
    } else {
        eprintln!(
            "[ATELIER_DEBUG_UPSTREAM] {} response JSON (truncated, total {} bytes):\n{}…",
            label,
            txt.len(),
            &txt[..MAX]
        );
    }
}

/// Logs the first `max` complete SSE events (useful to see whether `reasoning` / `reasoning_text` deltas appear).
fn debug_log_sse_event(emitted: &mut u32, max: u32, event: &str) {
    if !upstream_debug() || *emitted >= max {
        return;
    }
    *emitted += 1;
    let preview: String = event.chars().take(1200).collect();
    let suffix = if event.len() > 1200 { "…" } else { "" };
    eprintln!(
        "[ATELIER_DEBUG_UPSTREAM] SSE event {}/{} (preview):\n{}{}",
        *emitted,
        max,
        preview,
        suffix
    );
}

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

async fn generate_chat(
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let final_txt = if openrouter_compat {
        post_openrouter_chat(&client, &request, &mut body, &provider_label).await?
    } else {
        post_with_retries(&client, &request, &body, &provider_label).await?
    };

    parse_openai_like_response(&final_txt)
}

async fn generate_chat_stream(
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
    set_streaming(&mut body);
    let provider_label = provider_label(&request);
    let openrouter_compat = is_openrouter_endpoint(&request.provider.endpoint);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    if openrouter_compat {
        post_openrouter_chat_stream(&client, &request, &mut body, &provider_label, on_text_delta)
            .await
    } else {
        post_stream_with_retries(&client, &request, &body, &provider_label, on_text_delta).await
    }
}

async fn generate_responses(request: ChatRequest) -> AppResult<GenerateResponse> {
    let body = build_responses_body(&request);
    let provider_label = provider_label(&request);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let final_txt = post_with_retries(&client, &request, &body, &provider_label).await?;
    parse_responses_response(&final_txt)
}

async fn generate_responses_stream(
    request: ChatRequest,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut body = build_responses_body(&request);
    set_streaming(&mut body);
    let provider_label = provider_label(&request);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    post_responses_stream_with_retries(&client, &request, &body, &provider_label, on_text_delta)
        .await
}

async fn post_openrouter_chat(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &mut Value,
    provider_label: &str,
) -> AppResult<String> {
    let mut modality_stage: u8 = 0;
    'modalities: loop {
        apply_openrouter_modalities_stage(body, &request.model, modality_stage);

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
            let txt = resp.text().await?;
            if status.is_success() {
                debug_log_upstream_response_text(provider_label, &txt);
                return Ok(txt);
            }

            let msg = upstream_error_message(&txt);
            if attempt < MAX_ATTEMPTS && is_retryable_status(status) {
                sleep_for_attempt(attempt).await;
                continue;
            }
            if upstream_rejects_modalities(status, &msg) && modality_stage < 2 {
                modality_stage += 1;
                continue 'modalities;
            }
            return Err(AppError::Upstream(format!(
                "{} HTTP {}: {}",
                provider_label, status, msg
            )));
        }
        unreachable!("HTTP attempts should return or branch before completing the loop");
    }
}

async fn post_openrouter_chat_stream(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &mut Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut modality_stage: u8 = 0;
    'modalities: loop {
        apply_openrouter_modalities_stage(body, &request.model, modality_stage);

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
                        fallback_openrouter_chat_response(
                            client,
                            request,
                            body,
                            provider_label,
                            on_text_delta.clone(),
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
                return fallback_openrouter_chat_response(
                    client,
                    request,
                    body,
                    provider_label,
                    on_text_delta.clone(),
                )
                .await;
            }
            if upstream_rejects_modalities(status, &msg) && modality_stage < 2 {
                modality_stage += 1;
                continue 'modalities;
            }
            return Err(AppError::Upstream(format!(
                "{} HTTP {}: {}",
                provider_label, status, msg
            )));
        }
        unreachable!("HTTP attempts should return or branch before completing the loop");
    }
}

async fn post_stream_with_retries(
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

async fn post_responses_stream_with_retries(
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
            return match parse_responses_success(resp, on_text_delta.clone()).await {
                Ok(r) => Ok(r),
                Err(e) if is_empty_stream_upstream_error(&e) => {
                    fallback_responses_response(
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
            return fallback_responses_response(
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

async fn post_with_retries(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
) -> AppResult<String> {
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
        let txt = resp.text().await?;
        if status.is_success() {
            debug_log_upstream_response_text(provider_label, &txt);
            return Ok(txt);
        }

        let msg = upstream_error_message(&txt);
        if attempt < MAX_ATTEMPTS && is_retryable_status(status) {
            sleep_for_attempt(attempt).await;
            continue;
        }
        return Err(AppError::Upstream(format!(
            "{} HTTP {}: {}",
            provider_label, status, msg
        )));
    }
    unreachable!("HTTP attempts should return or branch before completing the loop");
}

async fn parse_openai_chat_success(
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

async fn parse_responses_success(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    if is_json_response(&resp) {
        let final_txt = resp.text().await?;
        if upstream_debug() {
            debug_log_upstream_response_text("responses API success (JSON, not SSE)", &final_txt);
        }
        let parsed = parse_responses_response(&final_txt)?;
        emit_final_text_if_needed(&parsed, &on_text_delta);
        return Ok(parsed);
    }
    consume_responses_stream(resp, on_text_delta).await
}

async fn consume_openai_chat_stream(
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
        let chunk = chunk?;
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

async fn consume_responses_stream(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut images = Vec::new();
    let mut usage = TokenUsage::default();
    let mut final_response: Option<Value> = None;
    let mut sse_debug_emitted = 0u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.extend_from_slice(&chunk);
        while let Some((event_end, sep_len)) = find_sse_event_end(&buffer) {
            let drained: Vec<u8> = buffer.drain(..event_end + sep_len).collect();
            let event = String::from_utf8_lossy(&drained[..event_end]);
            debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
            handle_responses_sse_event(
                &event,
                &mut text,
                &mut thinking,
                &mut images,
                &mut usage,
                &mut final_response,
                &on_text_delta,
            )?;
        }
    }

    if !buffer.is_empty() {
        let event = String::from_utf8_lossy(&buffer);
        debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
        handle_responses_sse_event(
            &event,
            &mut text,
            &mut thinking,
            &mut images,
            &mut usage,
            &mut final_response,
            &on_text_delta,
        )?;
    }

    if let Some(response) = final_response {
        let mut final_images = Vec::new();
        collect_response_images(&response, &mut final_images);
        if !final_images.is_empty() {
            images = final_images;
        }

        let final_usage = tokens::extract_usage(&response);
        merge_usage(&mut usage, final_usage);

        if let Some(extra) = extract_responses_reasoning(&response) {
            if thinking.trim().is_empty() {
                thinking = extra;
            } else if !extra.trim().is_empty() {
                thinking.push_str("\n\n");
                thinking.push_str(&extra);
            }
        }

        if text.trim().is_empty() {
            if let Some(final_text) = extract_responses_text(&response) {
                (on_text_delta)(StreamDelta::text(final_text.clone()));
                text = final_text;
            }
        }
    }

    // OpenAI Responses API surfaces tool calls inside the final response
    // object (`response.output[*].type == "function_call"`), not inside
    // streamed `delta.tool_calls`. They're parsed at the call site that
    // consumes `final_response`; the chat-completions accumulator here
    // is unused for this path.
    finalize_stream_response(text, thinking, images, usage, Vec::new())
}

fn set_streaming(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.insert("stream".into(), Value::Bool(true));
    }
}

fn is_json_response(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("application/json") && !v.contains("text/event-stream")
        })
        .unwrap_or(false)
}

fn emit_final_text_if_needed(resp: &GenerateResponse, on_text_delta: &TextDeltaCallback) {
    if let Some(text) = resp.text.as_deref() {
        if !text.is_empty() {
            (on_text_delta)(StreamDelta::text(text.to_string()));
        }
    }
}

async fn fallback_openrouter_chat_response(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut body = without_streaming(body);
    let final_txt = post_openrouter_chat(client, request, &mut body, provider_label).await?;
    let resp = parse_openai_like_response(&final_txt)?;
    emit_final_text_if_needed(&resp, &on_text_delta);
    Ok(resp)
}

async fn fallback_openai_chat_response(
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

async fn fallback_responses_response(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let body = without_streaming(body);
    let final_txt = post_with_retries(client, request, &body, provider_label).await?;
    let resp = parse_responses_response(&final_txt)?;
    emit_final_text_if_needed(&resp, &on_text_delta);
    Ok(resp)
}

fn without_streaming(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(map) = body.as_object_mut() {
        map.remove("stream");
    }
    body
}

fn find_sse_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buffer.len().saturating_sub(3) {
        if &buffer[i..i + 4] == b"\r\n\r\n" {
            return Some((i, 4));
        }
    }
    for i in 0..buffer.len().saturating_sub(1) {
        if &buffer[i..i + 2] == b"\n\n" {
            return Some((i, 2));
        }
    }
    None
}

fn sse_data_payload(event: &str) -> Option<String> {
    let mut data = Vec::new();
    for raw_line in event.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() {
        None
    } else {
        Some(data.join("\n"))
    }
}

fn handle_openai_chat_sse_event(
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
    merge_tool_call_deltas(&v, tool_calls);
    merge_usage(usage, tokens::extract_usage(&v));
    Ok(false)
}

fn handle_responses_sse_event(
    event: &str,
    text: &mut String,
    thinking: &mut String,
    images: &mut Vec<ImageResult>,
    usage: &mut TokenUsage,
    final_response: &mut Option<Value>,
    on_text_delta: &TextDeltaCallback,
) -> AppResult<()> {
    let Some(data) = sse_data_payload(event) else {
        return Ok(());
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }

    let v: Value = serde_json::from_str(data).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream SSE event: {err}; data={data}"
        ))
    })?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    if let Some(delta) = responses_stream_text_delta(&v) {
        text.push_str(&delta);
        (on_text_delta)(StreamDelta::text(delta));
    }
    if let Some(delta) = responses_stream_reasoning_delta(&v) {
        thinking.push_str(&delta);
        emit_thinking_deltas(on_text_delta, &delta);
    }

    collect_response_images(&v, images);
    merge_usage(usage, tokens::extract_usage(&v));
    if let Some(response) = v.get("response").cloned() {
        merge_usage(usage, tokens::extract_usage(&response));
        if v.get("type")
            .and_then(Value::as_str)
            .map(|typ| typ == "response.completed")
            .unwrap_or(false)
        {
            *final_response = Some(response);
        }
    }
    Ok(())
}

fn responses_stream_text_delta(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    if matches!(typ, "response.output_text.delta" | "response.refusal.delta") {
        return v.get("delta").and_then(Value::as_str).map(str::to_string);
    }
    None
}

fn responses_stream_reasoning_delta(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    if typ == "response.reasoning_text.delta" {
        return v.get("delta").and_then(Value::as_str).map(str::to_string);
    }
    None
}

fn extract_openai_chat_stream_update(v: &Value) -> (String, Vec<ImageResult>, String) {
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

fn collect_openai_delta_reasoning(v: &Value, out: &mut Vec<String>) {
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

fn collect_openai_delta_text(v: &Value, out: &mut Vec<String>) {
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

fn collect_content_part_text(v: &Value, out: &mut Vec<String>) {
    if let Some(s) = v.get("text").and_then(Value::as_str) {
        out.push(s.to_string());
        return;
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        out.push(s.to_string());
    }
}

fn merge_usage(target: &mut TokenUsage, next: TokenUsage) {
    if next.prompt_tokens.is_some() {
        target.prompt_tokens = next.prompt_tokens;
    }
    if next.completion_tokens.is_some() {
        target.completion_tokens = next.completion_tokens;
    }
    if next.total_tokens.is_some() {
        target.total_tokens = next.total_tokens;
    }
}

/// Buffered shape used while streaming tool_calls arrive piecewise.
/// OpenAI chat completions stream tool_calls as a sequence of deltas
/// indexed by `index`: the first delta carries `id` / `type` /
/// `function.name`, later deltas append more `function.arguments`
/// fragments. We accumulate them here and emit a final
/// [`crate::ai::chat::ProviderToolCall`] list when the stream ends.
#[derive(Debug, Default, Clone)]
struct PendingStreamToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Merge `choices[*].delta.tool_calls[*]` from one SSE event into the
/// running accumulator. Tool calls are addressed by `index` (OpenAI
/// guarantees stable indices across chunks for the same call).
fn merge_tool_call_deltas(v: &Value, out: &mut Vec<PendingStreamToolCall>) {
    let Some(choices) = v.get("choices").and_then(Value::as_array) else {
        return;
    };
    for choice in choices {
        let Some(arr) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for tc in arr {
            let idx = tc
                .get("index")
                .and_then(Value::as_u64)
                .map(|i| i as usize)
                .unwrap_or_else(|| out.len());
            if idx >= out.len() {
                out.resize_with(idx + 1, PendingStreamToolCall::default);
            }
            let slot = &mut out[idx];
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                if !id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                if !name.is_empty() {
                    slot.name = name.to_string();
                }
            }
            if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                if !args.is_empty() {
                    slot.arguments.push_str(args);
                }
            }
        }
    }
}

/// Convert the buffered stream-side tool-call state into the provider-
/// agnostic shape the agent loop expects. Drops entries that never
/// received a name (defensive against malformed streams).
fn finalize_pending_tool_calls(
    pending: Vec<PendingStreamToolCall>,
) -> Vec<crate::ai::chat::ProviderToolCall> {
    pending
        .into_iter()
        .filter(|p| !p.name.is_empty())
        .map(|p| {
            let raw_args = if p.arguments.trim().is_empty() {
                "{}"
            } else {
                p.arguments.as_str()
            };
            let arguments = serde_json::from_str::<Value>(raw_args).unwrap_or(Value::Null);
            crate::ai::chat::ProviderToolCall {
                id: p.id,
                name: p.name,
                arguments,
            }
        })
        .collect()
}

fn finalize_stream_response(
    text: String,
    thinking: String,
    mut images: Vec<ImageResult>,
    usage: TokenUsage,
    pending_tool_calls: Vec<PendingStreamToolCall>,
) -> AppResult<GenerateResponse> {
    let tool_calls = finalize_pending_tool_calls(pending_tool_calls);
    if upstream_debug() {
        eprintln!(
            "[ATELIER_DEBUG_UPSTREAM] stream assembled: text_chars={} thinking_chars={} image_count={} tool_calls={} usage={:?}",
            text.chars().count(),
            thinking.chars().count(),
            images.len(),
            tool_calls.len(),
            usage
        );
    }
    if !text.is_empty() {
        collect_inline_data_urls(&text, &mut images);
    }
    let text = if text.trim().is_empty() {
        None
    } else {
        Some(text)
    };
    let thinking_content = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    if images.is_empty()
        && text.as_deref().map(str::is_empty).unwrap_or(true)
        && thinking_content.as_deref().map(str::is_empty).unwrap_or(true)
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(
            "upstream stream did not contain generated image, text, or tool_calls".into(),
        ));
    }
    Ok(GenerateResponse {
        images,
        text,
        thinking_content,
        usage,
        tool_calls,
    })
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

fn build_chat_body(request: &ChatRequest, allow_image_parts: bool) -> Value {
    let user_content = chat_content(
        &request.prompt,
        &request.attachments,
        allow_image_parts,
        true,
    );

    let mut messages: Vec<Value> = Vec::new();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    for turn in &request.history {
        if let Some(message) = history_turn_to_chat_message(turn, allow_image_parts) {
            messages.push(message);
        }
    }
    messages.push(json!({ "role": "user", "content": user_content }));

    // Echo back the assistant turn that emitted the tool calls so
    // OpenAI's call/response symmetry check passes.
    if let Some(pending) = &request.pending_assistant_turn {
        let text = pending.text.as_deref().unwrap_or("");
        let tool_calls: Vec<Value> = pending
            .tool_calls
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": {
                        "name": c.name,
                        "arguments": serde_json::to_string(&c.arguments).unwrap_or_else(|_| "{}".into()),
                    }
                })
            })
            .collect();
        let mut msg = json!({ "role": "assistant" });
        let m = msg.as_object_mut().unwrap();
        if !text.is_empty() {
            m.insert("content".into(), Value::String(text.to_string()));
        } else {
            m.insert("content".into(), Value::Null);
        }
        if !tool_calls.is_empty() {
            m.insert("tool_calls".into(), Value::Array(tool_calls));
        }
        // Always echo reasoning_content when present: DeepSeek thinking mode is
        // enabled by default and requires it to be passed back whenever tool
        // calls were made, regardless of whether thinking was explicitly
        // requested by the client.
        if let Some(t) = pending
            .thinking_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            m.insert("reasoning_content".into(), json!(t));
        }
        messages.push(msg);
    }

    // Append any tool_result replies. OpenAI expects them after the
    // assistant message that emitted the corresponding tool_calls.
    for tr in &request.tool_results {
        let content = match &tr.content {
            Value::String(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        };
        messages.push(json!({
            "role": "tool",
            "tool_call_id": tr.tool_call_id,
            "content": content,
        }));
    }

    let mut body = json!({
        "model": request.model,
        "messages": messages,
    });

    let map = body.as_object_mut().unwrap();
    request.parameters.apply_model_params(map);
    request.parameters.apply_openai_reasoning_effort(map);
    request.parameters.apply_openai_compat_thinking_object(
        map,
        &request.model,
        &request.provider.endpoint,
    );
    if is_openrouter_endpoint(&request.provider.endpoint) {
        if let Some(image_config) = request.parameters.image_config() {
            map.insert("image_config".into(), image_config);
        }
    }

    // Surface available tools to the model. Image-generation flows
    // leave `tools` empty so the field is omitted.
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.schema,
                    }
                })
            })
            .collect();
        map.insert("tools".into(), Value::Array(tools));
    }

    body
}

fn chat_content(
    text: &str,
    attachments: &[AttachmentBytes],
    allow_image_parts: bool,
    include_empty_text: bool,
) -> Value {
    if attachments.is_empty() || !allow_image_parts {
        return Value::String(text.to_string());
    }

    let mut arr: Vec<Value> = Vec::with_capacity(attachments.len() + 1);
    if include_empty_text || !text.trim().is_empty() {
        arr.push(json!({"type":"text","text":text}));
    }
    for attachment in attachments {
        arr.push(json!({
            "type":"image_url",
            "image_url": { "url": data_url(attachment) }
        }));
    }
    Value::Array(arr)
}

fn history_turn_to_chat_message(turn: &HistoryTurn, allow_image_parts: bool) -> Option<Value> {
    let role = turn.role.trim();
    if role.is_empty() {
        return None;
    }

    let text = turn
        .text
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let role_allows_images = role == "user";
    let content = chat_content(
        &text,
        if role_allows_images {
            &turn.images
        } else {
            &[]
        },
        allow_image_parts,
        false,
    );
    if matches!(&content, Value::String(s) if s.trim().is_empty()) {
        return None;
    }

    let mut msg = json!({ "role": role, "content": content });
    // DeepSeek (and compatible providers) require `reasoning_content` to be
    // echoed back in assistant history turns when the original response
    // included it; omitting it causes a 400 error.
    if role == "assistant" {
        if let Some(t) = turn
            .thinking_content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            msg.as_object_mut()
                .unwrap()
                .insert("reasoning_content".into(), json!(t));
        }
    }
    Some(msg)
}

fn build_responses_body(request: &ChatRequest) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for turn in &request.history {
        if let Some(message) = history_turn_to_responses_message(turn) {
            input.push(message);
        }
    }
    input.push(responses_message(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));

    let mut body = json!({
        "model": request.model,
        "input": input,
    });
    let map = body.as_object_mut().unwrap();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        map.insert("instructions".into(), Value::String(sys.to_string()));
    }
    apply_responses_params(map, request);
    body
}

fn history_turn_to_responses_message(turn: &HistoryTurn) -> Option<Value> {
    let text = turn.text.as_deref();
    if text.map(|s| s.trim().is_empty()).unwrap_or(true) && turn.images.is_empty() {
        return None;
    }
    let role = match turn.role.as_str() {
        "assistant" => "assistant",
        "system" => "system",
        _ => "user",
    };
    Some(responses_message(role, text, &turn.images))
}

fn responses_message(role: &str, text: Option<&str>, attachments: &[AttachmentBytes]) -> Value {
    let mut content: Vec<Value> = Vec::new();
    if let Some(text) = text {
        if !text.trim().is_empty() {
            content.push(json!({ "type": "input_text", "text": text }));
        }
    }
    for attachment in attachments {
        content.push(json!({
            "type": "input_image",
            "image_url": data_url(attachment),
            "detail": "auto"
        }));
    }
    json!({
        "type": "message",
        "role": role,
        "content": content,
    })
}

fn apply_responses_params(body: &mut Map<String, Value>, request: &ChatRequest) {
    let params = &request.parameters;
    if let Some(v) = params.model.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if let Some(v) = params.model.top_p {
        body.insert("top_p".into(), json!(v));
    }
    if let Some(v) = params.model.max_tokens {
        body.insert("max_output_tokens".into(), json!(v));
    }
    params.apply_openai_reasoning_effort(body);
    params.apply_openai_compat_thinking_object(body, &request.model, &request.provider.endpoint);
}

fn data_url(att: &AttachmentBytes) -> String {
    format!("data:{};base64,{}", att.mime, B64.encode(&att.bytes))
}

fn is_openrouter_endpoint(endpoint: &str) -> bool {
    endpoint
        .trim()
        .to_ascii_lowercase()
        .contains("openrouter.ai")
}

fn requested_modalities(model: &str) -> Value {
    if is_image_only_model(model) {
        json!(["image"])
    } else {
        json!(["image", "text"])
    }
}

fn apply_openrouter_modalities_stage(body: &mut Value, model: &str, stage: u8) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    match stage {
        0 => {
            map.insert("modalities".into(), requested_modalities(model));
        }
        1 => {
            map.insert("modalities".into(), json!(["image"]));
        }
        _ => {
            map.remove("modalities");
        }
    }
}

fn upstream_rejects_modalities(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    (status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST) && m.contains("modalit")
}

fn is_empty_stream_upstream_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::Upstream(s) if s.contains("upstream stream did not contain")
    )
}

fn upstream_rejects_streaming(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    matches!(
        status,
        StatusCode::BAD_REQUEST
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::NOT_IMPLEMENTED
            | StatusCode::UNPROCESSABLE_ENTITY
    ) && (m.contains("stream") || m.contains("sse"))
}

fn is_image_only_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.starts_with("black-forest-labs/")
        || m.starts_with("bytedance-seed/")
        || m.starts_with("sourceful/")
        || m.starts_with("recraft/")
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
    )
}

fn should_retry_transport(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

async fn sleep_for_attempt(attempt: usize) {
    let backoff_ms = 500u64 * (1u64 << (attempt - 1));
    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
}

fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .or_else(|| v.pointer("/error/type"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

fn parse_openai_like_response(final_txt: &str) -> AppResult<GenerateResponse> {
    if final_txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }

    let v: Value = parse_response_json(final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let images = extract_images(&v);
    let text = extract_openai_chat_text(&v);
    let thinking_content = extract_openai_chat_thinking(&v);
    let tool_calls = extract_openai_chat_tool_calls(&v);
    let has_text = text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_thinking = thinking_content
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if images.is_empty() && !has_text && tool_calls.is_empty() && !has_thinking {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text, tool_calls, or reasoning. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        text,
        thinking_content,
        usage: tokens::extract_usage(&v),
        tool_calls,
    })
}

fn parse_responses_response(final_txt: &str) -> AppResult<GenerateResponse> {
    if final_txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }

    let v: Value = parse_response_json(final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let mut images = Vec::new();
    collect_response_images(&v, &mut images);
    let text = extract_responses_text(&v);
    let thinking_content = extract_responses_reasoning(&v);
    let has_text = text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_thinking = thinking_content
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if images.is_empty() && !has_text && !has_thinking {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text, or reasoning. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        text,
        thinking_content,
        usage: tokens::extract_usage(&v),
        tool_calls: Vec::new(),
    })
}

fn parse_response_json(txt: &str) -> AppResult<Value> {
    serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })
}

fn top_level_error_message(v: &Value) -> Option<String> {
    let error = v.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .unwrap_or("unknown upstream error");
    let code = error
        .get("code")
        .or_else(|| error.get("type"))
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| x.to_string())
        })
        .filter(|s| !s.trim().is_empty() && s != "null");

    Some(match code {
        Some(code) => format!("upstream error {}: {}", code, message),
        None => format!("upstream error: {}", message),
    })
}

fn extract_openai_chat_tool_calls(v: &Value) -> Vec<crate::ai::chat::ProviderToolCall> {
    let Some(arr) = v.pointer("/choices/0/message/tool_calls").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|tc| {
            let id = tc.get("id").and_then(Value::as_str)?.to_string();
            let name = tc
                .pointer("/function/name")
                .and_then(Value::as_str)?
                .to_string();
            let raw_args = tc
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments = serde_json::from_str::<Value>(raw_args).unwrap_or(Value::Null);
            Some(crate::ai::chat::ProviderToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

fn extract_openai_chat_text(v: &Value) -> Option<String> {
    let msg = v.pointer("/choices/0/message")?;

    if let Some(s) = msg.get("content").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        let mut parts: Vec<String> = Vec::new();
        for it in arr {
            let typ = it.get("type").and_then(Value::as_str).unwrap_or("");
            if typ == "reasoning" {
                continue;
            }
            if let Some(s) = it.get("text").and_then(Value::as_str) {
                if !s.trim().is_empty() {
                    parts.push(s.trim().to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }

    None
}

fn extract_openai_chat_thinking(v: &Value) -> Option<String> {
    let msg = v.pointer("/choices/0/message")?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = msg.get("reasoning").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(s) = msg.get("reasoning_content").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        for it in arr {
            if it.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            if let Some(s) = it
                .get("text")
                .or_else(|| it.get("reasoning"))
                .and_then(Value::as_str)
            {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn extract_responses_text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("output_text").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let mut parts = Vec::new();
    collect_text_values(v.get("output").unwrap_or(v), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn extract_responses_reasoning(v: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_reasoning_values(v.get("output").unwrap_or(v), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn collect_reasoning_values(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_reasoning_values(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if typ.contains("reasoning") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
            for key in ["content", "summary"] {
                if let Some(value) = map.get(key) {
                    collect_reasoning_values(value, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_text_values(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_text_values(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(typ, "output_text" | "text" | "summary_text") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
            for key in ["content", "summary"] {
                if let Some(value) = map.get(key) {
                    collect_text_values(value, out);
                }
            }
        }
        _ => {}
    }
}

fn parse_data_url(url: &str) -> Option<ImageResult> {
    let prefix = "data:";
    if !url.starts_with(prefix) {
        return None;
    }
    let rest = &url[prefix.len()..];
    let comma = rest.find(',')?;
    let header = &rest[..comma];
    let payload = &rest[comma + 1..];
    let mut mime = "image/png".to_string();
    let mut is_b64 = false;
    for part in header.split(';') {
        if part == "base64" {
            is_b64 = true;
        } else if part.starts_with("image/") {
            mime = part.to_string();
        }
    }
    if !is_b64 {
        return None;
    }
    match B64.decode(payload.as_bytes()) {
        Ok(bytes) => Some(ImageResult { bytes, mime }),
        Err(_) => None,
    }
}

fn parse_b64_image(payload: &str, mime: Option<&str>) -> Option<ImageResult> {
    B64.decode(payload.as_bytes())
        .ok()
        .map(|bytes| ImageResult {
            bytes,
            mime: mime.unwrap_or("image/png").to_string(),
        })
}

fn image_url_from_value(value: &Value) -> Option<&str> {
    [
        value.pointer("/image_url/url"),
        value.pointer("/imageUrl/url"),
        value.get("url"),
        value.get("image_url"),
        value.get("imageUrl"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
}

fn image_from_value(value: &Value) -> Option<ImageResult> {
    // Gemini generateContent / OpenRouter-normalized multimodal image parts
    if let Some(inline) = value.get("inline_data").or_else(|| value.get("inlineData")) {
        let mime = inline
            .get("mime_type")
            .or_else(|| inline.get("mimeType"))
            .and_then(Value::as_str);
        if let Some(data) = inline.get("data").and_then(Value::as_str) {
            if let Some(r) = parse_b64_image(data, mime) {
                return Some(r);
            }
        }
    }
    if let Some(u) = image_url_from_value(value) {
        if let Some(r) = parse_data_url(u) {
            return Some(r);
        }
    }
    let mime = value
        .get("mime_type")
        .or_else(|| value.get("mimeType"))
        .and_then(Value::as_str);
    value
        .get("b64_json")
        .or_else(|| value.get("base64"))
        .or_else(|| value.get("data"))
        .or_else(|| value.get("result"))
        .and_then(Value::as_str)
        .and_then(|payload| parse_b64_image(payload, mime))
}

fn extract_images(v: &Value) -> Vec<ImageResult> {
    let mut out = Vec::new();
    let msg = match v.pointer("/choices/0/message") {
        Some(x) => x,
        None => return out,
    };

    if let Some(arr) = msg.get("images").and_then(Value::as_array) {
        for it in arr {
            if let Some(r) = image_from_value(it) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        for part in arr {
            if let Some(r) = image_from_value(part) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(s) = msg.get("content").and_then(Value::as_str) {
        collect_inline_data_urls(s, &mut out);
    }
    out
}

fn collect_response_images(v: &Value, out: &mut Vec<ImageResult>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_response_images(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(image) = image_from_value(v) {
                out.push(image);
                return;
            }
            if let Some(s) = map.get("text").and_then(Value::as_str) {
                collect_inline_data_urls(s, out);
            }
            for key in ["output", "content", "images"] {
                if let Some(value) = map.get(key) {
                    collect_response_images(value, out);
                }
            }
        }
        Value::String(s) => collect_inline_data_urls(s, out),
        _ => {}
    }
}

fn collect_inline_data_urls(s: &str, out: &mut Vec<ImageResult>) {
    let needle = "data:image/";
    let mut i = 0;
    while let Some(start) = s[i..].find(needle).map(|p| p + i) {
        let tail = &s[start..];
        let end_rel = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == ')' || c == '\'')
            .unwrap_or(tail.len());
        let url = &tail[..end_rel];
        if let Some(r) = parse_data_url(url) {
            out.push(r);
        }
        i = start + end_rel.max(1);
        if i >= s.len() {
            break;
        }
    }
}

fn push_response_detail(details: &mut Vec<String>, label: &str, value: &Value) {
    let raw = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return;
    }
    let short: String = trimmed.chars().take(240).collect();
    details.push(format!("{}={}", label, short));
}

fn push_response_detail_at(details: &mut Vec<String>, v: &Value, path: &str, label: &str) {
    if let Some(value) = v.pointer(path) {
        push_response_detail(details, label, value);
    }
}

fn empty_response_details(v: &Value) -> String {
    let mut details = Vec::new();

    push_response_detail_at(&mut details, v, "/choices/0/finish_reason", "finish_reason");
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/native_finish_reason",
        "native_finish_reason",
    );
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/error/code",
        "choice_error_code",
    );
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/error/message",
        "choice_error_message",
    );
    push_response_detail_at(&mut details, v, "/choices/0/message/refusal", "refusal");
    push_response_detail_at(&mut details, v, "/error/code", "error_code");
    push_response_detail_at(&mut details, v, "/error/message", "error_message");
    push_response_detail_at(&mut details, v, "/status", "status");
    push_response_detail_at(
        &mut details,
        v,
        "/incomplete_details/reason",
        "incomplete_reason",
    );

    if v.pointer("/choices/0/message").is_none() && v.get("output").is_none() {
        details.push("missing choices[0].message or output".to_string());
    }

    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}

#[allow(dead_code)]
fn _usage_from_openai(v: &Value) -> TokenUsage {
    tokens::extract_usage(v)
}
