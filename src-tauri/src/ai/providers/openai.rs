use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use crate::ai::chat::{
    emit_text_deltas, emit_thinking_deltas, AttachmentBytes, ChatRequest, GenerateResponse,
    HistoryTurn, ImageResult, PendingAssistantTurn, StreamDelta, TextDeltaCallback,
    ToolResultMessage,
};
use crate::ai::providers::{ChatProvider, ProviderFuture, OPENAI_RESPONSES_SDK, OPENAI_SDK};
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::{AppError, AppResult};

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
    let body_str = serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string());
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
        *emitted, max, preview, suffix
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

    let client = crate::ai::providers::build_chat_client()?;

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

async fn generate_responses(request: ChatRequest) -> AppResult<GenerateResponse> {
    let provider_label = provider_label(&request);
    let client = crate::ai::providers::build_chat_client()?;
    let mut request = request;
    let body = build_responses_body(&request);
    match post_with_retries(&client, &request, &body, &provider_label).await {
        Ok(final_txt) => parse_responses_response(&final_txt),
        Err(err) if should_reset_responses_cache(&err, &request) => {
            request.previous_response_id = None;
            let body = build_responses_body(&request);
            let final_txt = post_with_retries(&client, &request, &body, &provider_label).await?;
            parse_responses_response(&final_txt)
        }
        Err(err) => Err(err),
    }
}

async fn generate_responses_stream(
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

fn should_reset_responses_cache(err: &AppError, request: &ChatRequest) -> bool {
    if !responses_cache_active(request)
        || request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        return false;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("previous_response")
        || msg.contains("response_id")
        || msg.contains("caching")
        || msg.contains("context cache")
        || msg.contains("not found")
        || msg.contains("expired")
        || msg.contains("invalid")
}

async fn post_openrouter_chat(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &mut Value,
    provider_label: &str,
) -> AppResult<String> {
    let mut modality_stage = openrouter_initial_modality_stage(request);
    let mut tools_stripped = false;
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
            if upstream_rejects_tools(status, &msg) && !tools_stripped {
                tools_stripped = true;
                strip_tools_from_body(body);
                continue 'modalities;
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
    let mut modality_stage = openrouter_initial_modality_stage(request);
    let mut tools_stripped = false;
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
            if upstream_rejects_tools(status, &msg) && !tools_stripped {
                tools_stripped = true;
                strip_tools_from_body(body);
                continue 'modalities;
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
    let mut pending_tools: Vec<PendingStreamToolCall> = Vec::new();
    let mut sse_debug_emitted = 0u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(stream_read_error)?;
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
                &mut pending_tools,
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
            &mut pending_tools,
            &on_text_delta,
        )?;
    }

    let mut response_id = None;
    let mut tool_calls = finalize_pending_tool_calls(pending_tools);
    if let Some(response) = final_response {
        let mut final_images = Vec::new();
        collect_response_images(&response, &mut final_images);
        if !final_images.is_empty() {
            images = final_images;
        }

        let final_usage = tokens::extract_usage(&response);
        merge_usage(&mut usage, final_usage);

        // Prefer emitting recovered reasoning before any late text so UIs that
        // only append (without reorder) still see think→answer order.
        if let Some(extra) = extract_responses_reasoning(&response) {
            let extra = extra.trim();
            if !extra.is_empty() {
                if thinking.trim().is_empty() {
                    thinking = extra.to_string();
                    emit_thinking_deltas(&on_text_delta, &thinking);
                } else if !thinking.contains(extra) {
                    thinking.push_str("\n\n");
                    thinking.push_str(extra);
                }
            }
        }

        if text.trim().is_empty() {
            if let Some(final_text) = extract_responses_text(&response) {
                emit_text_deltas(&on_text_delta, &final_text);
                text = final_text;
            }
        } else if let Some(final_text) = extract_responses_text(&response) {
            // Cover any trailing body that only appeared on `response.completed`.
            apply_committed_text(&mut text, &final_text, &on_text_delta);
        }

        // Prefer live-accumulated tool calls; fall back to the completed payload
        // when the stream skipped function_call argument events entirely.
        if tool_calls.is_empty() {
            tool_calls = extract_responses_tool_calls(&response);
        }
        response_id = response
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    }

    if !text.is_empty() {
        collect_inline_data_urls(&text, &mut images);
    }
    let text_opt = if text.trim().is_empty() {
        None
    } else {
        Some(text)
    };
    let thinking_opt = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    if images.is_empty()
        && text_opt.as_deref().map(str::is_empty).unwrap_or(true)
        && thinking_opt.as_deref().map(str::is_empty).unwrap_or(true)
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(
            "upstream stream did not contain generated image, text, or tool_calls".into(),
        ));
    }
    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text: text_opt,
        thinking_content: thinking_opt,
        usage,
        tool_calls,
        response_id,
    })
}

fn set_streaming(body: &mut Value, include_usage: bool) {
    if let Some(map) = body.as_object_mut() {
        map.insert("stream".into(), Value::Bool(true));
        // Chat Completions (incl. Volcengine Ark / Doubao): streaming omits
        // token usage unless explicitly requested. The final SSE chunk then
        // carries `usage` with an empty `choices` array.
        // Docs: https://console.volcengine.com/ark/region:cn-beijing/docs/82379/1569618
        if include_usage {
            map.insert(
                "stream_options".into(),
                json!({ "include_usage": true }),
            );
        }
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
        map.remove("stream_options");
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
    sse_event_name_and_data(event).map(|(_, data)| data)
}

/// Parse SSE `event:` / `data:` lines. Ark may put the event type only on the
/// `event:` line while `data:` omits `type`.
fn sse_event_name_and_data(event: &str) -> Option<(Option<String>, String)> {
    let mut event_name = None;
    let mut data = Vec::new();
    for raw_line in event.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(rest) = line.strip_prefix("event:") {
            let name = rest.strip_prefix(' ').unwrap_or(rest).trim();
            if !name.is_empty() {
                event_name = Some(name.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() {
        None
    } else {
        Some((event_name, data.join("\n")))
    }
}

fn ensure_responses_event_type(v: &mut Value, event_name: Option<&str>) {
    let Some(name) = event_name.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    // SSE `event:` is authoritative. Ark sometimes omits `type` in `data:`,
    // or puts a shorter alias that would miss our matchers.
    obj.insert("type".into(), Value::String(name.to_string()));
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
    merge_tool_call_deltas(&v, tool_calls, on_text_delta);
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
    pending_tools: &mut Vec<PendingStreamToolCall>,
    on_text_delta: &TextDeltaCallback,
) -> AppResult<()> {
    let Some((event_name, data)) = sse_event_name_and_data(event) else {
        return Ok(());
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }

    let mut v: Value = serde_json::from_str(data).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream SSE event: {err}; data={data}"
        ))
    })?;
    ensure_responses_event_type(&mut v, event_name.as_deref());
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    // Live stream: reasoning deltas first (docs: response.reasoning_summary_text.delta).
    if let Some(delta) = responses_stream_reasoning_delta(&v) {
        thinking.push_str(&delta);
        emit_thinking_deltas(on_text_delta, &delta);
    } else if let Some(committed) = responses_stream_reasoning_committed(&v) {
        // Part/item done events: surface thinking as soon as the reasoning
        // item finishes, before output_text starts (even if deltas were skipped).
        apply_committed_thinking(thinking, &committed, on_text_delta);
    }
    if let Some(delta) = responses_stream_text_delta(&v) {
        text.push_str(&delta);
        emit_text_deltas(on_text_delta, &delta);
    } else if let Some(committed) = responses_stream_text_committed(&v) {
        // Ark may skip `output_text.delta` and only ship the body on
        // `output_text.done` / `content_part.done` / `output_item.done`.
        apply_committed_text(text, &committed, on_text_delta);
    }

    merge_responses_tool_events(&v, pending_tools, on_text_delta);

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
    if is_responses_output_text_delta(typ) {
        return sse_delta_text(v);
    }
    None
}

fn is_responses_output_text_delta(typ: &str) -> bool {
    matches!(
        typ,
        "response.output_text.delta"
            | "response.refusal.delta"
            | "output_text.delta"
            | "refusal.delta"
    ) || typ.ends_with(".output_text.delta")
        || typ.ends_with(".refusal.delta")
}

/// Non-delta events that still carry finished assistant body mid-stream.
fn responses_stream_text_committed(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    match typ {
        "response.output_text.done" | "response.refusal.done" | "output_text.done" => v
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty()),
        "response.content_part.done" | "content_part.done" => {
            let part = v.get("part")?;
            let part_typ = part.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(part_typ, "output_text" | "refusal" | "text") {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        }
        "response.output_item.done" | "output_item.done" => {
            let item = v.get("item")?;
            let item_typ = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_typ == "message" || item_typ.ends_with("message") {
                extract_responses_text(&json!({ "output": [item] }))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn responses_stream_reasoning_delta(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    // https://console.volcengine.com/ark/region:cn-beijing/docs/82379/1599499
    if matches!(
        typ,
        "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning.delta"
    ) {
        return sse_delta_text(v);
    }
    None
}

/// Non-delta events that still carry finished reasoning text mid-stream.
fn responses_stream_reasoning_committed(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    match typ {
        "response.reasoning_summary_text.done" | "response.reasoning_text.done" => v
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        "response.reasoning_summary_part.done" => v
            .pointer("/part/text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        "response.content_part.done" => {
            let part = v.get("part")?;
            let part_typ = part.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(part_typ, "summary_text" | "reasoning_text") {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            } else {
                None
            }
        }
        "response.output_item.done" => {
            let item = v.get("item")?;
            let item_typ = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_typ == "reasoning" || item_typ.starts_with("reasoning") {
                extract_responses_reasoning(&json!({ "output": [item] }))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn sse_delta_text(v: &Value) -> Option<String> {
    match v.get("delta") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Object(map)) => map
            .get("text")
            .or_else(|| map.get("content"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        // Some gateways put the fragment in top-level `text` even on *.delta.
        _ => v
            .get("text")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

fn apply_committed_thinking(
    thinking: &mut String,
    committed: &str,
    on_text_delta: &TextDeltaCallback,
) {
    let committed = committed.trim();
    if committed.is_empty() {
        return;
    }
    if thinking.is_empty() {
        *thinking = committed.to_string();
        emit_thinking_deltas(on_text_delta, thinking);
        return;
    }
    if committed.starts_with(thinking.as_str()) {
        let rest = &committed[thinking.len()..];
        if !rest.is_empty() {
            thinking.push_str(rest);
            emit_thinking_deltas(on_text_delta, rest);
        }
        return;
    }
    // Deltas already covered this snapshot (or a superset).
    if thinking.contains(committed) || committed.contains(thinking.as_str()) {
        return;
    }
}

fn apply_committed_text(
    text: &mut String,
    committed: &str,
    on_text_delta: &TextDeltaCallback,
) {
    if committed.is_empty() {
        return;
    }
    if text.is_empty() {
        *text = committed.to_string();
        emit_text_deltas(on_text_delta, text);
        return;
    }
    if committed.starts_with(text.as_str()) {
        let rest = &committed[text.len()..];
        if !rest.is_empty() {
            text.push_str(rest);
            emit_text_deltas(on_text_delta, rest);
        }
        return;
    }
    if text.contains(committed) || committed.contains(text.as_str()) {
        return;
    }
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
    if next.cache_read_tokens.is_some() {
        target.cache_read_tokens = next.cache_read_tokens;
    }
    if next.cache_write_tokens.is_some() {
        target.cache_write_tokens = next.cache_write_tokens;
    }
}

/// Buffered shape used while streaming tool_calls arrive piecewise.
/// OpenAI chat completions stream tool_calls as a sequence of deltas
/// indexed by `index`: the first delta carries `id` / `type` /
/// `function.name`, later deltas append more `function.arguments`
/// fragments. Responses API uses `item_id` / `output_index` instead.
/// We accumulate them here and emit a final
/// [`crate::ai::chat::ProviderToolCall`] list when the stream ends.
#[derive(Debug, Default, Clone)]
struct PendingStreamToolCall {
    /// Call id forwarded to the UI / tool loop (`call_id` on Responses).
    id: String,
    /// Responses output-item id (`item.id`); used to match argument deltas.
    item_id: String,
    name: String,
    arguments: String,
}

/// Merge `choices[*].delta.tool_calls[*]` from one SSE event into the
/// running accumulator. Tool calls are addressed by `index` (OpenAI
/// guarantees stable indices across chunks for the same call).
///
/// Besides buffering for the final [`crate::ai::chat::ProviderToolCall`],
/// each fragment is forwarded via `on_text_delta` as a
/// [`StreamDelta::tool_call`] so the renderer can display the tool input
/// (e.g. a document's `content`) as it streams in, before the turn ends.
fn merge_tool_call_deltas(
    v: &Value,
    out: &mut Vec<PendingStreamToolCall>,
    on_text_delta: &TextDeltaCallback,
) {
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
            let mut identity_changed = false;
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                if !id.is_empty() {
                    slot.id = id.to_string();
                    identity_changed = true;
                }
            }
            if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                if !name.is_empty() {
                    slot.name = name.to_string();
                    identity_changed = true;
                }
            }
            let mut fragment = String::new();
            if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                if !args.is_empty() {
                    slot.arguments.push_str(args);
                    fragment.push_str(args);
                }
            }
            // Forward the live fragment once we know the call id. An empty
            // `fragment` with a fresh id/name still emits, so the UI can
            // create the pending card before any argument bytes arrive.
            if !slot.id.is_empty() && (identity_changed || !fragment.is_empty()) {
                (on_text_delta)(StreamDelta::tool_call(
                    slot.id.clone(),
                    slot.name.clone(),
                    fragment,
                ));
            }
        }
    }
}

/// Responses API tool streaming:
/// `output_item.added` (function_call) → `function_call_arguments.delta`* →
/// `function_call_arguments.done` → `output_item.done`.
fn merge_responses_tool_events(
    v: &Value,
    out: &mut Vec<PendingStreamToolCall>,
    on_text_delta: &TextDeltaCallback,
) {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    if typ.ends_with("output_item.added") || typ.ends_with("output_item.done") {
        let Some(item) = v.get("item") else {
            return;
        };
        let item_typ = item.get("type").and_then(Value::as_str).unwrap_or("");
        if item_typ != "function_call" {
            return;
        }
        let output_index = v
            .get("output_index")
            .and_then(Value::as_u64)
            .map(|i| i as usize);
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| item.get("id").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let idx = find_responses_tool_slot(out, output_index, &item_id, &call_id);
        if idx >= out.len() {
            out.resize_with(idx + 1, PendingStreamToolCall::default);
        }
        let slot = &mut out[idx];
        let mut identity_changed = false;
        if !item_id.is_empty() && slot.item_id != item_id {
            slot.item_id = item_id;
            identity_changed = true;
        }
        if !call_id.is_empty() && slot.id != call_id {
            slot.id = call_id;
            identity_changed = true;
        }
        if !name.is_empty() && slot.name != name {
            slot.name = name;
            identity_changed = true;
        }

        let mut fragment = String::new();
        if !args.is_empty() {
            if slot.arguments.is_empty() {
                slot.arguments = args.clone();
                fragment = args;
            } else if args.starts_with(slot.arguments.as_str()) {
                fragment = args[slot.arguments.len()..].to_string();
                if !fragment.is_empty() {
                    slot.arguments.push_str(&fragment);
                }
            } else if slot.arguments.starts_with(args.as_str()) {
                // Already have a superset from deltas.
            } else if !slot.arguments.contains(&args) {
                // Replace with the authoritative snapshot from done/added.
                fragment = args.clone();
                slot.arguments = args;
            }
        }

        if slot.id.is_empty() || slot.name.is_empty() {
            return;
        }
        if identity_changed {
            // Open the card; replay any args buffered before name/call_id landed.
            emit_tool_arg_deltas(on_text_delta, &slot.id, &slot.name, &slot.arguments);
        } else if !fragment.is_empty() {
            emit_tool_arg_deltas(on_text_delta, &slot.id, &slot.name, &fragment);
        }
        return;
    }

    if typ.ends_with("function_call_arguments.delta") {
        let fragment = v
            .get("delta")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("");
        if fragment.is_empty() {
            return;
        }
        let item_id = v.get("item_id").and_then(Value::as_str).unwrap_or("");
        let output_index = v
            .get("output_index")
            .and_then(Value::as_u64)
            .map(|i| i as usize);
        let idx = find_responses_tool_slot(out, output_index, item_id, item_id);
        if idx >= out.len() {
            out.resize_with(idx + 1, PendingStreamToolCall::default);
        }
        let slot = &mut out[idx];
        if slot.item_id.is_empty() && !item_id.is_empty() {
            slot.item_id = item_id.to_string();
        }
        if slot.id.is_empty() && !item_id.is_empty() {
            // Until output_item.added supplies call_id, use item_id so the UI
            // can open a pending card; later identity updates keep the same slot.
            slot.id = item_id.to_string();
        }
        slot.arguments.push_str(fragment);
        // Frontend ignores tool deltas until `name` is known.
        if !slot.id.is_empty() && !slot.name.is_empty() {
            emit_tool_arg_deltas(on_text_delta, &slot.id, &slot.name, fragment);
        }
        return;
    }

    if typ.ends_with("function_call_arguments.done") {
        let args = v
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("");
        if args.is_empty() {
            return;
        }
        let item_id = v.get("item_id").and_then(Value::as_str).unwrap_or("");
        let output_index = v
            .get("output_index")
            .and_then(Value::as_u64)
            .map(|i| i as usize);
        let idx = find_responses_tool_slot(out, output_index, item_id, item_id);
        if idx >= out.len() {
            out.resize_with(idx + 1, PendingStreamToolCall::default);
        }
        let slot = &mut out[idx];
        if slot.item_id.is_empty() && !item_id.is_empty() {
            slot.item_id = item_id.to_string();
        }
        if slot.id.is_empty() && !item_id.is_empty() {
            slot.id = item_id.to_string();
        }
        let fragment = if slot.arguments.is_empty() {
            slot.arguments = args.to_string();
            args.to_string()
        } else if args.starts_with(slot.arguments.as_str()) {
            let rest = args[slot.arguments.len()..].to_string();
            slot.arguments.push_str(&rest);
            rest
        } else {
            String::new()
        };
        if !slot.id.is_empty() && !slot.name.is_empty() && !fragment.is_empty() {
            emit_tool_arg_deltas(on_text_delta, &slot.id, &slot.name, &fragment);
        }
    }
}

fn find_responses_tool_slot(
    out: &[PendingStreamToolCall],
    output_index: Option<usize>,
    item_id: &str,
    call_id: &str,
) -> usize {
    if !item_id.is_empty() {
        if let Some(i) = out.iter().position(|p| {
            (!p.item_id.is_empty() && p.item_id == item_id)
                || (!p.id.is_empty() && (p.id == item_id || p.id == call_id))
        }) {
            return i;
        }
    }
    if !call_id.is_empty() {
        if let Some(i) = out.iter().position(|p| p.id == call_id) {
            return i;
        }
    }
    if let Some(idx) = output_index {
        return idx;
    }
    out.len()
}

/// Typewriter-split tool argument fragments so large CreateDoc/Write payloads
/// don't appear as a one-shot dump when Ark batches them.
fn emit_tool_arg_deltas(cb: &TextDeltaCallback, id: &str, name: &str, chunk: &str) {
    if id.is_empty() {
        return;
    }
    // Card creation with unknown name is deferred until name arrives.
    if name.is_empty() && chunk.is_empty() {
        return;
    }
    if chunk.is_empty() {
        (cb)(StreamDelta::tool_call(
            id.to_string(),
            name.to_string(),
            String::new(),
        ));
        return;
    }
    for ch in chunk.chars() {
        (cb)(StreamDelta::tool_call(
            id.to_string(),
            name.to_string(),
            ch.to_string(),
        ));
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
            let arguments = parse_tool_call_arguments(&p.name, &p.arguments);
            crate::ai::chat::ProviderToolCall {
                id: p.id,
                name: p.name,
                arguments,
            }
        })
        .collect()
}

/// Parse a tool call's accumulated `function.arguments` string.
///
/// Some upstreams (notably doubao/ark) occasionally emit argument JSON that
/// strict parsing rejects: raw control characters inside string values,
/// output truncated mid-stream, or the whole payload concatenated twice.
/// Dropping the call to `Value::Null` loses the entire generation, so on
/// strict-parse failure we attempt a best-effort repair before giving up.
fn parse_tool_call_arguments(tool_name: &str, raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(v) => v,
        Err(err) => match repair_tool_call_arguments(trimmed) {
            Some(v) => {
                eprintln!(
                    "[ATELIER_TOOL_ARGS] repaired malformed `{}` arguments ({} bytes; strict parse: {})",
                    tool_name,
                    trimmed.len(),
                    err
                );
                v
            }
            None => {
                eprintln!(
                    "[ATELIER_TOOL_ARGS] unrepairable `{}` arguments ({} bytes; strict parse: {})",
                    tool_name,
                    trimmed.len(),
                    err
                );
                Value::Null
            }
        },
    }
}

/// Best-effort repair of malformed tool-argument JSON. Returns `None` when
/// nothing parseable can be salvaged.
fn repair_tool_call_arguments(raw: &str) -> Option<Value> {
    // Duplicated / concatenated payloads (`{...}{...}`): strict parsing fails
    // with "trailing characters", but the first value alone is valid.
    if let Some(v) = first_json_value(raw) {
        return Some(v);
    }

    // Escape raw control chars and invalid escape sequences inside strings.
    let (clean, open_stack, in_string) = sanitize_json_fragment(raw);
    if let Some(v) = first_json_value(&clean) {
        return Some(v);
    }

    // Truncated output: close whatever is still open. Several closing
    // strategies are tried because the cut may fall mid-key, mid-value or
    // right after a separator.
    let closers: String = open_stack
        .iter()
        .rev()
        .map(|c| if *c == '{' { '}' } else { ']' })
        .collect();
    let mut candidates: Vec<String> = Vec::new();
    if in_string {
        // Cut inside a string value: close the quote.
        candidates.push(format!("{clean}\"{closers}"));
        // Cut inside an object key: close the quote and give it a value.
        candidates.push(format!("{clean}\":null{closers}"));
    } else {
        candidates.push(format!("{clean}{closers}"));
        // Cut right after `:` → the key still needs a value.
        candidates.push(format!("{clean}null{closers}"));
        // Cut right after `,` → drop the dangling separator.
        let stripped = clean.trim_end().trim_end_matches([',', ':']);
        candidates.push(format!("{stripped}{closers}"));
    }
    candidates
        .iter()
        .find_map(|cand| serde_json::from_str::<Value>(cand).ok())
}

/// Extract the first complete JSON value from `raw`, ignoring anything after it.
fn first_json_value(raw: &str) -> Option<Value> {
    let mut iter = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    match iter.next() {
        Some(Ok(v)) => Some(v),
        _ => None,
    }
}

/// Single pass over a (possibly malformed) JSON fragment:
/// - escapes raw control characters (U+0000..U+001F) inside strings;
/// - neutralizes invalid escape sequences (`\x`, incomplete `\uXX`) by
///   escaping the backslash itself;
/// - records which containers are still open and whether the fragment ends
///   inside a string, so the caller can synthesize closers.
fn sanitize_json_fragment(raw: &str) -> (String, Vec<char>, bool) {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if !in_string {
            match ch {
                '"' => {
                    in_string = true;
                    out.push(ch);
                }
                '{' | '[' => {
                    stack.push(ch);
                    out.push(ch);
                }
                '}' => {
                    if stack.last() == Some(&'{') {
                        stack.pop();
                    }
                    out.push(ch);
                }
                ']' => {
                    if stack.last() == Some(&'[') {
                        stack.pop();
                    }
                    out.push(ch);
                }
                _ => out.push(ch),
            }
            i += 1;
            continue;
        }
        match ch {
            '"' => {
                in_string = false;
                out.push(ch);
                i += 1;
            }
            '\\' => match chars.get(i + 1) {
                None => {
                    // Dangling escape at buffer end: drop it.
                    i += 1;
                }
                Some('u') => {
                    let hex_ok = chars.len() >= i + 6
                        && chars[i + 2..i + 6].iter().all(|c| c.is_ascii_hexdigit());
                    if hex_ok {
                        out.push('\\');
                        out.push('u');
                        out.extend(&chars[i + 2..i + 6]);
                        i += 6;
                    } else {
                        out.push_str("\\\\u");
                        i += 2;
                    }
                }
                Some(next @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                    out.push('\\');
                    out.push(*next);
                    i += 2;
                }
                Some(_) => {
                    // Invalid escape like `\x`: escape the backslash literally
                    // and reprocess the following char normally.
                    out.push_str("\\\\");
                    i += 1;
                }
            },
            c if (c as u32) < 0x20 => {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push_str(&format!("\\u{:04x}", c as u32)),
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    (out, stack, in_string)
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
        && thinking_content
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(
            "upstream stream did not contain generated image, text, or tool_calls".into(),
        ));
    }
    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text,
        thinking_content,
        usage,
        tool_calls,
        response_id: None,
    })
}

/// Context wrapper for a transport error hit *while consuming* the SSE
/// response body. reqwest's own message here is usually the opaque
/// "error decoding response body"; [`crate::error::describe_reqwest_error`]
/// unfolds the source chain so the real cause (connection reset, unexpected
/// EOF, idle keepalive failure, ...) is visible in the UI.
fn stream_read_error(err: reqwest::Error) -> AppError {
    AppError::Upstream(format!(
        "connection interrupted while streaming upstream response: {}",
        crate::error::describe_reqwest_error(&err)
    ))
}

/// Best-effort DELETE of a stored Responses API object (Session cache tip).
/// Failures are logged and ignored — local DB is the source of truth for clearing.
pub async fn delete_stored_response(endpoint: &str, api_key: &str, response_id: &str) {
    let id = response_id.trim();
    if id.is_empty() || api_key.trim().is_empty() {
        return;
    }
    let Some(url) = responses_object_url(endpoint, id) else {
        return;
    };
    let Ok(client) = crate::ai::providers::build_chat_client() else {
        return;
    };
    match client.delete(&url).bearer_auth(api_key).send().await {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => {}
        Ok(resp) => {
            eprintln!(
                "[atelier] delete stored response {} failed: HTTP {}",
                id,
                resp.status()
            );
        }
        Err(err) => {
            eprintln!("[atelier] delete stored response {id} failed: {err}");
        }
    }
}

fn responses_object_url(endpoint: &str, response_id: &str) -> Option<String> {
    let ep = endpoint.trim().trim_end_matches('/');
    if ep.is_empty() {
        return None;
    }
    // Typical: .../api/v3/responses  or  .../v1/responses
    if ep.ends_with("/responses") {
        return Some(format!("{ep}/{response_id}"));
    }
    // .../chat/completions → sibling /responses/{id}
    if let Some(base) = ep.strip_suffix("/chat/completions") {
        return Some(format!("{base}/responses/{response_id}"));
    }
    // Bare API root: .../api/v3
    Some(format!("{ep}/responses/{response_id}"))
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
        if turn.role == "assistant" && !turn.timeline.is_empty() {
            // Replay the prior assistant turn segment-by-segment so tool
            // history reads as native assistant{tool_calls} + role:"tool"
            // messages rather than a leak-prone plain-text transcript.
            // AgentStage markers are host-only and dropped.
            for seg in &turn.timeline {
                match seg {
                    crate::ai::chat::TimelineSegment::Text {
                        text,
                        thinking_content,
                    } => {
                        append_openai_assistant_text_turn(
                            &mut messages,
                            text,
                            thinking_content.as_deref(),
                        );
                    }
                    crate::ai::chat::TimelineSegment::ToolRound { .. } => {
                        if let Some(round) = seg.to_tool_round() {
                            append_openai_assistant_tool_turn(&mut messages, &round.assistant);
                            append_openai_tool_results(&mut messages, &round.results);
                        }
                    }
                    crate::ai::chat::TimelineSegment::AgentStage { .. } => {}
                }
            }
        } else if let Some(message) = history_turn_to_chat_message(turn, allow_image_parts) {
            messages.push(message);
        }
    }
    messages.push(json!({ "role": "user", "content": user_content }));

    for round in &request.tool_chain {
        append_openai_assistant_tool_turn(&mut messages, &round.assistant);
        append_openai_tool_results(&mut messages, &round.results);
    }
    if let Some(pending) = &request.pending_assistant_turn {
        append_openai_assistant_tool_turn(&mut messages, pending);
    }
    append_openai_tool_results(&mut messages, &request.tool_results);

    let mut body = json!({
        "model": request.model,
        "messages": messages,
    });

    let map = body.as_object_mut().unwrap();
    request.parameters.apply_model_params(map);
    request
        .parameters
        .apply_thinking_params(map, &request.provider.endpoint);
    if is_openrouter_endpoint(&request.provider.endpoint) && openrouter_wants_image_output(request)
    {
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

fn append_openai_assistant_text_turn(
    messages: &mut Vec<Value>,
    text: &str,
    thinking_content: Option<&str>,
) {
    let t = text.trim();
    let thinking = thinking_content.map(str::trim).filter(|s| !s.is_empty());
    if t.is_empty() && thinking.is_none() {
        return;
    }
    let mut msg = json!({ "role": "assistant" });
    let m = msg.as_object_mut().unwrap();
    if !t.is_empty() {
        m.insert("content".into(), Value::String(t.to_string()));
    } else {
        m.insert("content".into(), Value::Null);
    }
    if let Some(thinking) = thinking {
        m.insert("reasoning_content".into(), json!(thinking));
    }
    messages.push(msg);
}

fn append_openai_assistant_tool_turn(messages: &mut Vec<Value>, pending: &PendingAssistantTurn) {
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
    if let Some(t) = pending
        .thinking_content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        m.insert("reasoning_content".into(), json!(t));
    }
    if !text.is_empty() || !pending.tool_calls.is_empty() || pending.thinking_content.is_some() {
        messages.push(msg);
    }
}

fn append_openai_tool_results(messages: &mut Vec<Value>, tool_results: &[ToolResultMessage]) {
    for tr in tool_results {
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

fn responses_cache_active(request: &ChatRequest) -> bool {
    (request.context_cache_enabled || request.provider.context_cache_enabled)
        && crate::ai::parameters::is_volcengine_endpoint(&request.provider.endpoint)
}

fn build_responses_body(request: &ChatRequest) -> Value {
    let cache = responses_cache_active(request);
    let continuing = cache
        && request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();

    let input = if continuing {
        build_responses_delta_input(request)
    } else {
        build_responses_full_input(request, cache)
    };

    let mut body = json!({
        "model": request.model,
        "input": input,
    });
    let map = body.as_object_mut().unwrap();

    if continuing {
        if let Some(prev) = request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            map.insert("previous_response_id".into(), Value::String(prev.to_string()));
        }
    } else if !cache {
        // Non-cache mode keeps system prompt in `instructions`.
        let sys = request.system_prompt.trim();
        if !sys.is_empty() {
            map.insert("instructions".into(), Value::String(sys.to_string()));
        }
    }

    // Tools only on chain head (Volcengine Session cache constraint).
    if !continuing && !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                })
            })
            .collect();
        map.insert("tools".into(), Value::Array(tools));
    }

    if cache {
        map.insert("store".into(), Value::Bool(true));
        map.insert("caching".into(), json!({ "type": "enabled" }));
        let expire_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 86_400)
            .unwrap_or(0);
        if expire_at > 0 {
            map.insert("expire_at".into(), json!(expire_at));
        }
    }

    apply_responses_params(map, request);
    body
}

/// Full request input: system (cache head) + history + user + in-turn tool rounds.
fn build_responses_full_input(request: &ChatRequest, cache_head: bool) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();
    if cache_head {
        let sys = request.system_prompt.trim();
        if !sys.is_empty() {
            input.push(responses_message("system", Some(sys), &[]));
        }
    }
    for turn in &request.history {
        append_responses_history_turn(&mut input, turn);
    }
    input.push(responses_message(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));
    for round in &request.tool_chain {
        append_responses_tool_round(&mut input, round);
    }
    if let Some(pending) = &request.pending_assistant_turn {
        append_responses_pending_assistant(&mut input, pending);
    }
    append_responses_tool_results(&mut input, &request.tool_results);
    input
}

/// Cache continuation: only the latest tool outputs, or the new user turn.
fn build_responses_delta_input(request: &ChatRequest) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();
    if let Some(pending) = &request.pending_assistant_turn {
        // Should be rare (engine commits before the next call); keep for safety.
        append_responses_tool_results(&mut input, &request.tool_results);
        if input.is_empty() && !pending.tool_calls.is_empty() {
            append_responses_pending_assistant(&mut input, pending);
            append_responses_tool_results(&mut input, &request.tool_results);
        }
        if !input.is_empty() {
            return input;
        }
    }
    if let Some(round) = request.tool_chain.last() {
        append_responses_tool_results(&mut input, &round.results);
        if !input.is_empty() {
            return input;
        }
    }
    // New user turn continuing a prior session cache chain.
    input.push(responses_message(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));
    input
}

fn append_responses_history_turn(input: &mut Vec<Value>, turn: &HistoryTurn) {
    if turn.role == "assistant" && !turn.timeline.is_empty() {
        for seg in &turn.timeline {
            match seg {
                crate::ai::chat::TimelineSegment::Text { text, .. } => {
                    if !text.trim().is_empty() {
                        input.push(responses_message("assistant", Some(text), &[]));
                    }
                }
                crate::ai::chat::TimelineSegment::ToolRound { .. } => {
                    if let Some(round) = seg.to_tool_round() {
                        append_responses_tool_round(input, &round);
                    }
                }
                crate::ai::chat::TimelineSegment::AgentStage { .. } => {}
            }
        }
        return;
    }
    if let Some(message) = history_turn_to_responses_message(turn) {
        input.push(message);
    }
}

fn append_responses_tool_round(input: &mut Vec<Value>, round: &crate::ai::chat::ToolChainRound) {
    append_responses_pending_assistant(input, &round.assistant);
    append_responses_tool_results(input, &round.results);
}

fn append_responses_pending_assistant(input: &mut Vec<Value>, pending: &PendingAssistantTurn) {
    if let Some(text) = pending.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        input.push(responses_message("assistant", Some(text), &[]));
    }
    for call in &pending.tool_calls {
        input.push(json!({
            "type": "function_call",
            "call_id": call.id,
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".into()),
        }));
    }
}

fn append_responses_tool_results(
    input: &mut Vec<Value>,
    tool_results: &[crate::ai::chat::ToolResultMessage],
) {
    for tr in tool_results {
        let output = match &tr.content {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        input.push(json!({
            "type": "function_call_output",
            "call_id": tr.tool_call_id,
            "output": output,
        }));
    }
}

fn extract_responses_tool_calls(v: &Value) -> Vec<crate::ai::chat::ProviderToolCall> {
    let Some(arr) = v.get("output").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let typ = item.get("type").and_then(Value::as_str).unwrap_or("");
            if typ != "function_call" {
                return None;
            }
            let id = item
                .get("call_id")
                .and_then(Value::as_str)
                .or_else(|| item.get("id").and_then(Value::as_str))?
                .to_string();
            let name = item.get("name").and_then(Value::as_str)?.to_string();
            let raw_args = item
                .get("arguments")
                .and_then(|a| {
                    if let Some(s) = a.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(a.to_string())
                    }
                })
                .unwrap_or_else(|| "{}".into());
            let arguments = serde_json::from_str(&raw_args).unwrap_or_else(|_| json!({}));
            Some(crate::ai::chat::ProviderToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
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
    // Volcengine Ark Responses uses `reasoning.effort` (minimal/low/medium/
    // high/max). Chat Completions fields `thinking` / `reasoning_effort`
    // are rejected on this path.
    if crate::ai::parameters::is_volcengine_endpoint(&request.provider.endpoint) {
        params.apply_volcengine_responses_reasoning(body);
    } else {
        params.apply_thinking_params(body, &request.provider.endpoint);
    }
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

/// Agent tool-calling requests must not ask OpenRouter for image output.
fn openrouter_wants_image_output(request: &ChatRequest) -> bool {
    if !request.tools.is_empty() {
        return false;
    }
    if is_image_only_model(&request.model) {
        return true;
    }
    if request.parameters.image_config().is_some() {
        return true;
    }
    request.model.trim().to_ascii_lowercase().contains("image")
}

fn openrouter_initial_modality_stage(request: &ChatRequest) -> u8 {
    if openrouter_wants_image_output(request) {
        0
    } else {
        2
    }
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

/// OpenRouter returns HTTP 404 with a message like
/// "No endpoints found that support tool use." when the selected
/// model/provider combination doesn't support function calling. We
/// detect this and retry without `tools` so the agent can still
/// produce a plain-text response rather than surfacing a hard error.
fn upstream_rejects_tools(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    matches!(status, StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST)
        && (m.contains("tool use")
            || m.contains("tool_use")
            || m.contains("function call")
            || m.contains("invalid_argument"))
}

fn strip_tools_from_body(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("tools");
        map.remove("tool_choice");
    }
}

fn is_empty_stream_upstream_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::Upstream(s) if s.contains("upstream stream did not contain")
    )
}

fn upstream_rejects_streaming(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    if !matches!(
        status,
        StatusCode::BAD_REQUEST
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::NOT_IMPLEMENTED
            | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return false;
    }
    // Ignore unknown-parameter complaints about `stream_options` — that means
    // we should drop the field, not disable streaming entirely.
    if m.contains("stream_options") {
        return false;
    }
    m.contains("stream") || m.contains("sse")
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
        videos: Vec::new(),
        text,
        thinking_content,
        usage: tokens::extract_usage(&v),
        tool_calls,
        response_id: None,
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
    let tool_calls = extract_responses_tool_calls(&v);
    let response_id = v
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let has_text = text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_thinking = thinking_content
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if images.is_empty() && !has_text && !has_thinking && tool_calls.is_empty() {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text, tool_calls, or reasoning. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text,
        thinking_content,
        usage: tokens::extract_usage(&v),
        tool_calls,
        response_id,
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
    let Some(arr) = v
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    else {
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
            let arguments = parse_tool_call_arguments(&name, raw_args);
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
            // Reasoning item: `{ type: "reasoning", summary: [{type:summary_text,text}], content? }`
            if typ == "reasoning" || typ.starts_with("reasoning") {
                push_trimmed_text(map.get("text"), out);
                if let Some(summary) = map.get("summary") {
                    collect_reasoning_text_parts(summary, out);
                }
                if let Some(content) = map.get("content") {
                    collect_reasoning_text_parts(content, out);
                }
                return;
            }
            if matches!(typ, "summary_text" | "reasoning_text") {
                push_trimmed_text(map.get("text"), out);
            }
        }
        _ => {}
    }
}

fn collect_reasoning_text_parts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_reasoning_text_parts(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(typ, "summary_text" | "reasoning_text" | "") {
                push_trimmed_text(map.get("text"), out);
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
        _ => {}
    }
}

fn push_trimmed_text(v: Option<&Value>, out: &mut Vec<String>) {
    if let Some(text) = v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        out.push(text.to_string());
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
            // Reasoning items / summary_text belong to thinking, never assistant body.
            if typ == "reasoning" || typ.starts_with("reasoning") || typ == "summary_text" {
                return;
            }
            if matches!(typ, "output_text" | "text") {
                push_trimmed_text(map.get("text"), out);
            }
            if let Some(content) = map.get("content") {
                collect_text_values(content, out);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_args_valid_json_passes_through() {
        let v = parse_tool_call_arguments("CreateDoc", r#"{"title":"第二章","content":"正文"}"#);
        assert_eq!(v["title"], "第二章");
        assert_eq!(v["content"], "正文");
    }

    #[test]
    fn parse_tool_args_empty_becomes_object() {
        assert_eq!(parse_tool_call_arguments("CreateDoc", "   "), json!({}));
    }

    #[test]
    fn repair_recovers_raw_newlines_in_strings() {
        let raw = "{\"title\":\"第二章\",\"content\":\"第一行\n\t第二行\"}";
        let v = parse_tool_call_arguments("CreateDoc", raw);
        assert_eq!(v["title"], "第二章");
        assert_eq!(v["content"], "第一行\n\t第二行");
    }

    #[test]
    fn responses_object_url_from_responses_endpoint() {
        assert_eq!(
            responses_object_url("https://ark.cn-beijing.volces.com/api/v3/responses", "resp_1")
                .as_deref(),
            Some("https://ark.cn-beijing.volces.com/api/v3/responses/resp_1")
        );
    }

    #[test]
    fn responses_cache_continue_omits_tools_and_instructions() {
        let mut request = ChatRequest {
            provider: crate::ai::chat::ProviderConfig {
                id: "p".into(),
                name: "p".into(),
                sdk: OPENAI_RESPONSES_SDK.into(),
                endpoint: "https://ark.cn-beijing.volces.com/api/v3/responses".into(),
                api_key: "k".into(),
                context_cache_enabled: true,
            },
            model: "doubao-seed".into(),
            prompt: "follow up".into(),
            attachments: Vec::new(),
            system_prompt: "you are helpful".into(),
            history: Vec::new(),
            parameters: crate::ai::parameters::factory().build(
                "auto".into(),
                "auto".into(),
                crate::data::settings::ModelParamSettings {
                    thinking_enabled: Some(true),
                    thinking_effort: Some("high".into()),
                    ..Default::default()
                },
            ),
            tools: vec![crate::ai::chat::ToolDefinition {
                name: "Bash".into(),
                description: "run".into(),
                schema: json!({ "type": "object" }),
            }],
            tool_chain: Vec::new(),
            tool_results: Vec::new(),
            pending_assistant_turn: None,
            previous_response_id: Some("resp_prev".into()),
            context_cache_enabled: true,
        };
        let body = build_responses_body(&request);
        assert_eq!(body["previous_response_id"], "resp_prev");
        assert!(body.get("tools").is_none());
        assert!(body.get("instructions").is_none());
        assert_eq!(body["caching"]["type"], "enabled");
        assert_eq!(body["input"][0]["role"], "user");
        // Ark Responses: thinking.type + reasoning.effort (not Chat Completions fields).
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert!(body["reasoning"].get("summary").is_none());

        request.previous_response_id = None;
        let head = build_responses_body(&request);
        assert!(head.get("previous_response_id").is_none());
        assert!(head.get("tools").is_some());
        assert!(head.get("instructions").is_none()); // cache head uses system message
        assert_eq!(head["input"][0]["role"], "system");
        assert!(head.get("reasoning_effort").is_none());
        assert_eq!(head["thinking"]["type"], "enabled");
        assert_eq!(head["reasoning"]["effort"], "high");
        assert!(head["reasoning"].get("summary").is_none());
    }

    #[test]
    fn responses_reasoning_summary_delta_is_thinking() {
        let v = json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "先判断意图"
        });
        assert_eq!(
            responses_stream_reasoning_delta(&v).as_deref(),
            Some("先判断意图")
        );
        assert!(responses_stream_text_delta(&v).is_none());
    }

    #[test]
    fn responses_output_text_delta_and_done_are_body() {
        let delta = json!({
            "type": "response.output_text.delta",
            "delta": "你好"
        });
        assert_eq!(responses_stream_text_delta(&delta).as_deref(), Some("你好"));
        assert!(responses_stream_reasoning_delta(&delta).is_none());

        let done = json!({
            "type": "response.output_text.done",
            "text": "你好，世界"
        });
        assert_eq!(
            responses_stream_text_committed(&done).as_deref(),
            Some("你好，世界")
        );

        let item_done = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "完整正文" }]
            }
        });
        assert_eq!(
            responses_stream_text_committed(&item_done).as_deref(),
            Some("完整正文")
        );
    }

    #[test]
    fn responses_reasoning_output_item_done_is_committed() {
        let v = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "推理摘要" }]
            }
        });
        assert_eq!(
            responses_stream_reasoning_committed(&v).as_deref(),
            Some("推理摘要")
        );
    }

    #[test]
    fn sse_event_line_fills_missing_type() {
        let raw = "event: response.reasoning_summary_text.delta\ndata: {\"delta\":\"and\"}\n\n";
        let (name, data) = sse_event_name_and_data(raw).unwrap();
        assert_eq!(name.as_deref(), Some("response.reasoning_summary_text.delta"));
        let mut v: Value = serde_json::from_str(&data).unwrap();
        ensure_responses_event_type(&mut v, name.as_deref());
        assert_eq!(
            responses_stream_reasoning_delta(&v).as_deref(),
            Some("and")
        );
    }

    #[test]
    fn responses_extract_keeps_summary_out_of_body() {
        let v = json!({
            "output": [
                {
                    "type": "reasoning",
                    "summary": [{ "type": "summary_text", "text": "这是思考" }]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "这是正文" }]
                }
            ]
        });
        assert_eq!(extract_responses_reasoning(&v).as_deref(), Some("这是思考"));
        assert_eq!(extract_responses_text(&v).as_deref(), Some("这是正文"));
    }

    #[test]
    fn responses_function_call_arguments_stream_live() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let cb: TextDeltaCallback = Arc::new(move |d| {
            if let Some(tc) = d.tool_call {
                seen_cb
                    .lock()
                    .unwrap()
                    .push((tc.id, tc.name, tc.arguments));
            }
        });
        let mut pending = Vec::new();

        merge_responses_tool_events(
            &json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "id": "fc_1",
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "CreateDoc",
                    "arguments": "",
                    "status": "in_progress"
                }
            }),
            &mut pending,
            &cb,
        );
        merge_responses_tool_events(
            &json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "output_index": 0,
                "delta": "{\"title\":\""
            }),
            &mut pending,
            &cb,
        );
        merge_responses_tool_events(
            &json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_1",
                "output_index": 0,
                "delta": "第二章\"}"
            }),
            &mut pending,
            &cb,
        );
        merge_responses_tool_events(
            &json!({
                "type": "response.function_call_arguments.done",
                "item_id": "fc_1",
                "output_index": 0,
                "arguments": "{\"title\":\"第二章\"}"
            }),
            &mut pending,
            &cb,
        );

        let calls = finalize_pending_tool_calls(pending);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "CreateDoc");
        assert_eq!(calls[0].arguments["title"], "第二章");

        let events = seen.lock().unwrap();
        assert!(events.iter().any(|e| e.0 == "call_1" && e.1 == "CreateDoc"));
        let streamed: String = events
            .iter()
            .filter(|e| e.0 == "call_1")
            .map(|e| e.2.as_str())
            .collect();
        assert!(streamed.contains("第二章"));
    }

    #[test]
    fn extract_responses_function_calls() {
        let v = json!({
            "id": "resp_x",
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "Bash",
                    "arguments": "{\"command\":\"ls\"}"
                }
            ]
        });
        let calls = extract_responses_tool_calls(&v);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "Bash");
        assert_eq!(calls[0].arguments["command"], "ls");
    }

    #[test]
    fn repair_recovers_duplicated_payload() {
        let raw = r#"{"title":"a","content":"b"}{"title":"a","content":"b"}"#;
        let v = parse_tool_call_arguments("CreateDoc", raw);
        assert_eq!(v["title"], "a");
        assert_eq!(v["content"], "b");
    }

    #[test]
    fn repair_recovers_truncated_string_value() {
        let raw = r#"{"title":"第二章","content":"写到一半突然断"#;
        let v = parse_tool_call_arguments("CreateDoc", raw);
        assert_eq!(v["title"], "第二章");
        assert_eq!(v["content"], "写到一半突然断");
    }

    #[test]
    fn repair_recovers_cut_after_colon_and_comma() {
        let after_colon = r#"{"title":"a","content":"#;
        let v = parse_tool_call_arguments("CreateDoc", after_colon);
        assert_eq!(v["title"], "a");
        assert_eq!(v["content"], Value::Null);

        let after_comma = r#"{"title":"a","content":"b","#;
        let v = parse_tool_call_arguments("CreateDoc", after_comma);
        assert_eq!(v["title"], "a");
        assert_eq!(v["content"], "b");
    }

    #[test]
    fn repair_recovers_truncated_mid_key() {
        let raw = r#"{"title":"a","cont"#;
        let v = parse_tool_call_arguments("CreateDoc", raw);
        assert_eq!(v["title"], "a");
    }

    #[test]
    fn repair_neutralizes_invalid_escapes() {
        let raw = r#"{"title":"a\x1","content":"b\u12"}"#;
        let v = parse_tool_call_arguments("CreateDoc", raw);
        assert_eq!(v["title"], "a\\x1");
        assert_eq!(v["content"], "b\\u12");
    }

    #[test]
    fn unrepairable_garbage_falls_back_to_null() {
        assert_eq!(
            parse_tool_call_arguments("CreateDoc", "not json at all"),
            Value::Null
        );
    }

    #[test]
    fn assistant_text_turn_echoes_reasoning_content() {
        let mut messages = Vec::new();
        append_openai_assistant_text_turn(&mut messages, "最终答复", Some("先推理"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"], "最终答复");
        assert_eq!(messages[0]["reasoning_content"], "先推理");
    }

    #[test]
    fn assistant_text_turn_thinking_only() {
        let mut messages = Vec::new();
        append_openai_assistant_text_turn(&mut messages, "", Some("只思考"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], Value::Null);
        assert_eq!(messages[0]["reasoning_content"], "只思考");
    }

    #[test]
    fn set_streaming_requests_include_usage_for_chat() {
        let mut body = json!({ "model": "doubao", "messages": [] });
        set_streaming(&mut body, true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
    }

    #[test]
    fn set_streaming_can_omit_stream_options() {
        let mut body = json!({ "model": "gpt", "input": [] });
        set_streaming(&mut body, false);
        assert_eq!(body["stream"], true);
        assert!(body.get("stream_options").is_none());
    }

    #[test]
    fn responses_streaming_omits_stream_options() {
        let mut body = json!({ "model": "doubao", "input": [] });
        // Responses path must keep usage via `response.completed`, not chat
        // `stream_options` (rejected by Ark / OpenAI Responses).
        set_streaming(&mut body, false);
        assert_eq!(body["stream"], true);
        assert!(body.get("stream_options").is_none());
    }

    #[test]
    fn upstream_rejects_streaming_ignores_stream_options_param_errors() {
        assert!(!upstream_rejects_streaming(
            StatusCode::BAD_REQUEST,
            "Unknown parameter: 'stream_options'",
        ));
        assert!(upstream_rejects_streaming(
            StatusCode::BAD_REQUEST,
            "streaming is not supported for this model",
        ));
    }

    #[test]
    fn without_streaming_strips_stream_options() {
        let body = json!({
            "stream": true,
            "stream_options": { "include_usage": true },
            "model": "x",
        });
        let cleaned = without_streaming(&body);
        assert!(cleaned.get("stream").is_none());
        assert!(cleaned.get("stream_options").is_none());
        assert_eq!(cleaned["model"], "x");
    }

    #[test]
    fn merge_usage_from_final_empty_choices_chunk() {
        let mut usage = TokenUsage::default();
        // Intermediate chunk with usage: null — no overwrite.
        merge_usage(
            &mut usage,
            tokens::extract_usage(&json!({
                "choices": [{ "delta": { "content": "hi" } }],
                "usage": null,
            })),
        );
        assert!(usage.prompt_tokens.is_none());

        // Final Ark/OpenAI usage chunk: empty choices + usage object.
        merge_usage(
            &mut usage,
            tokens::extract_usage(&json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 34,
                    "total_tokens": 46
                }
            })),
        );
        assert_eq!(usage.prompt_tokens, Some(12));
        assert_eq!(usage.completion_tokens, Some(34));
        assert_eq!(usage.total_tokens, Some(46));
    }
}
