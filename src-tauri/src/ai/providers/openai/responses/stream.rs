use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::ai::chat::{
    emit_text_deltas, emit_thinking_deltas, ChatRequest, GenerateResponse, ImageResult,
    TextDeltaCallback,
};
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::{AppError, AppResult};

use super::super::common::{
    collect_inline_data_urls, collect_response_images, debug_log_sse_event,
    debug_log_upstream_request, debug_log_upstream_response_text, emit_final_text_if_needed,
    emit_tool_arg_deltas, finalize_pending_tool_calls, find_sse_event_end,
    is_empty_stream_upstream_error, is_json_response, is_retryable_status, merge_usage,
    post_with_retries, should_retry_transport, sleep_for_attempt, sse_event_name_and_data,
    stream_read_error, top_level_error_message, upstream_debug, upstream_error_message,
    upstream_rejects_streaming, without_streaming, PendingStreamToolCall, MAX_ATTEMPTS,
};
use super::parse::{
    extract_responses_reasoning, extract_responses_text, extract_responses_tool_calls,
    parse_responses_response,
};

pub(crate) async fn post_responses_stream_with_retries(
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
pub(crate) async fn parse_responses_success(
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
pub(crate) async fn consume_responses_stream(
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
pub(crate) async fn fallback_responses_response(
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
pub(crate) fn ensure_responses_event_type(v: &mut Value, event_name: Option<&str>) {
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
pub(crate) fn handle_responses_sse_event(
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

pub(crate) fn responses_stream_text_delta(v: &Value) -> Option<String> {
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    if is_responses_output_text_delta(typ) {
        return sse_delta_text(v);
    }
    None
}

pub(crate) fn is_responses_output_text_delta(typ: &str) -> bool {
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
pub(crate) fn responses_stream_text_committed(v: &Value) -> Option<String> {
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

pub(crate) fn responses_stream_reasoning_delta(v: &Value) -> Option<String> {
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
pub(crate) fn responses_stream_reasoning_committed(v: &Value) -> Option<String> {
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

pub(crate) fn sse_delta_text(v: &Value) -> Option<String> {
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

pub(crate) fn apply_committed_thinking(
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

pub(crate) fn apply_committed_text(
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
/// Responses API tool streaming:
/// `output_item.added` (function_call) → `function_call_arguments.delta`* →
/// `function_call_arguments.done` → `output_item.done`.
pub(crate) fn merge_responses_tool_events(
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

pub(crate) fn find_responses_tool_slot(
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
