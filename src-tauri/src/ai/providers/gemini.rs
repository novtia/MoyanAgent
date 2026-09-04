use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use crate::ai::chat::{
    emit_thinking_deltas, AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ImageResult,
    PendingAssistantTurn, StreamDelta, TextDeltaCallback, ToolResultMessage,
};
use crate::ai::providers::openai::common::{
    debug_log_sse_event, debug_log_upstream_request, emit_tool_arg_deltas, find_sse_event_end,
    is_json_response, message_rejects_gemini_arg_streaming, sse_data_payload,
    should_retry_failed_stream_attempt, should_retry_http_error, should_retry_transport,
    sleep_for_attempt, stream_read_error, MAX_ATTEMPTS,
};
use crate::ai::providers::{ChatProvider, ProviderFuture, GEMINI_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

pub struct GeminiProvider;

impl GeminiProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for GeminiProvider {
    fn sdk(&self) -> &'static str {
        GEMINI_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move { generate_stream(request, on_text_delta).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let url = gemini_url(&request.provider.endpoint, &request.model, false);
    let body = build_body(&request, false);
    let provider_label = provider_label(&request);

    let client = crate::ai::providers::build_chat_client()?;

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &request.provider.api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "{} HTTP {}: {}",
            provider_label,
            status,
            upstream_error_message(&txt)
        )));
    }
    parse_response(&txt)
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

fn gemini_url(endpoint: &str, model: &str, stream: bool) -> String {
    let endpoint = endpoint.trim();
    let method = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    let mut url = if endpoint.contains("{model}") {
        replace_gemini_method(&endpoint.replace("{model}", model.trim()), method)
    } else if endpoint.contains(":generateContent") || endpoint.contains(":streamGenerateContent")
    {
        replace_gemini_method(endpoint, method)
    } else {
        format!(
            "{}/models/{}:{}",
            endpoint.trim_end_matches('/'),
            model.trim(),
            method
        )
    };
    if stream {
        url = with_query_param(&url, "alt", "sse");
    }
    url
}

fn replace_gemini_method(endpoint: &str, method: &str) -> String {
    let (path, query) = match endpoint.find('?') {
        Some(i) => (&endpoint[..i], &endpoint[i..]),
        None => (endpoint, ""),
    };
    let replaced = if let Some(idx) = path.find(":streamGenerateContent") {
        format!(
            "{}:{}{}",
            &path[..idx],
            method,
            &path[idx + ":streamGenerateContent".len()..]
        )
    } else if let Some(idx) = path.find(":generateContent") {
        format!(
            "{}:{}{}",
            &path[..idx],
            method,
            &path[idx + ":generateContent".len()..]
        )
    } else {
        path.to_string()
    };
    format!("{replaced}{query}")
}

fn with_query_param(url: &str, key: &str, value: &str) -> String {
    let needle = format!("{key}=");
    if url.contains(&needle) {
        return url.to_string();
    }
    if url.contains('?') {
        format!("{url}&{key}={value}")
    } else {
        format!("{url}?{key}={value}")
    }
}

fn build_body(request: &ChatRequest, stream_function_args: bool) -> Value {
    use crate::ai::chat::TimelineSegment;
    let mut contents: Vec<Value> = Vec::new();
    for turn in &request.history {
        if turn.role == "assistant" && !turn.timeline.is_empty() {
            // Replay the prior assistant turn segment-by-segment so tool
            // history reads as native functionCall / functionResponse parts
            // rather than a leak-prone plain-text transcript. AgentStage
            // markers are host-only and dropped.
            for seg in &turn.timeline {
                match seg {
                    TimelineSegment::Text { text, .. } => {
                        let t = text.trim();
                        if !t.is_empty() {
                            contents.push(content_from_parts("model", Some(t), &[]));
                        }
                    }
                    TimelineSegment::ToolRound { .. } => {
                        if let Some(round) = seg.to_tool_round() {
                            append_gemini_assistant_tool_turn(&mut contents, &round.assistant);
                            append_gemini_tool_results(
                                &mut contents,
                                &round.assistant,
                                &round.results,
                            );
                        }
                    }
                    TimelineSegment::AgentStage { .. } => {}
                }
            }
        } else if let Some(content) = history_turn_to_content(turn) {
            contents.push(content);
        }
    }
    contents.push(content_from_parts(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));

    for round in &request.tool_chain {
        append_gemini_assistant_tool_turn(&mut contents, &round.assistant);
        append_gemini_tool_results(&mut contents, &round.assistant, &round.results);
    }
    if let Some(pending) = &request.pending_assistant_turn {
        append_gemini_assistant_tool_turn(&mut contents, pending);
        append_gemini_tool_results(&mut contents, pending, &request.tool_results);
    }
    append_gemini_todo_snapshot(&mut contents, request);

    let mut body = json!({ "contents": contents });
    let map = body.as_object_mut().unwrap();

    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        map.insert(
            "system_instruction".into(),
            json!({ "parts": [{ "text": sys }] }),
        );
    }

    if !request.tools.is_empty() {
        let function_declarations: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                })
            })
            .collect();
        map.insert(
            "tools".into(),
            json!([{ "functionDeclarations": function_declarations }]),
        );
        if stream_function_args {
            map.insert(
                "toolConfig".into(),
                json!({
                    "functionCallingConfig": {
                        "streamFunctionCallArguments": true
                    }
                }),
            );
        }
    }

    let generation_config = generation_config(&request);
    if !generation_config.is_empty() {
        map.insert("generationConfig".into(), Value::Object(generation_config));
    }
    body
}

fn append_gemini_assistant_tool_turn(contents: &mut Vec<Value>, pending: &PendingAssistantTurn) {
    let mut parts: Vec<Value> = Vec::new();
    if let Some(text) = pending
        .text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        parts.push(json!({ "text": text }));
    }
    for tc in &pending.tool_calls {
        parts.push(json!({
            "functionCall": { "name": tc.name, "args": tc.arguments }
        }));
    }
    if !parts.is_empty() {
        contents.push(json!({ "role": "model", "parts": parts }));
    }
}

fn append_gemini_tool_results(
    contents: &mut Vec<Value>,
    assistant: &PendingAssistantTurn,
    tool_results: &[ToolResultMessage],
) {
    if tool_results.is_empty() {
        return;
    }
    let parts: Vec<Value> = tool_results
        .iter()
        .map(|tr| {
            let name = assistant
                .tool_calls
                .iter()
                .find(|c| c.id == tr.tool_call_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| tr.tool_call_id.clone());
            let response = match &tr.content {
                Value::Object(_) | Value::Array(_) => tr.content.clone(),
                other => json!({ "result": other }),
            };
            json!({
                "functionResponse": { "name": name, "response": response }
            })
        })
        .collect();
    contents.push(json!({ "role": "user", "parts": parts }));
}

fn append_gemini_todo_snapshot(contents: &mut Vec<Value>, request: &ChatRequest) {
    let Some(snap) = request.todo_snapshot_text() else {
        return;
    };
    if let Some(last) = contents.last_mut() {
        if last.get("role").and_then(|r| r.as_str()) == Some("user") {
            if let Some(parts) = last.get_mut("parts").and_then(|p| p.as_array_mut()) {
                parts.push(json!({ "text": snap }));
                return;
            }
        }
    }
    contents.push(json!({
        "role": "user",
        "parts": [{ "text": snap }]
    }));
}

fn history_turn_to_content(turn: &HistoryTurn) -> Option<Value> {
    let text = turn.text.as_deref();
    if text.map(|s| s.trim().is_empty()).unwrap_or(true) && turn.images.is_empty() {
        return None;
    }
    let role = if turn.role == "assistant" {
        "model"
    } else {
        "user"
    };
    let images: &[AttachmentBytes] = if role == "user" { &turn.images } else { &[] };
    Some(content_from_parts(role, text, images))
}

fn content_from_parts(role: &str, text: Option<&str>, attachments: &[AttachmentBytes]) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    if let Some(text) = text {
        if !text.trim().is_empty() {
            parts.push(json!({ "text": text }));
        }
    }
    for attachment in attachments {
        parts.push(json!({
            "inline_data": {
                "mime_type": attachment.mime.as_str(),
                "data": B64.encode(&attachment.bytes),
            }
        }));
    }
    json!({ "role": role, "parts": parts })
}

fn generation_config(request: &ChatRequest) -> Map<String, Value> {
    let mut out = Map::new();
    if request.model.to_ascii_lowercase().contains("image") {
        out.insert("responseModalities".into(), json!(["TEXT", "IMAGE"]));
        if request.parameters.aspect_ratio != "auto" {
            out.insert(
                "imageConfig".into(),
                json!({ "aspectRatio": request.parameters.aspect_ratio.as_str() }),
            );
        }
    }
    if let Some(v) = request.parameters.model.temperature {
        out.insert("temperature".into(), json!(v));
    }
    if let Some(v) = request.parameters.model.top_p {
        out.insert("topP".into(), json!(v));
    }
    if let Some(v) = request.parameters.model.max_tokens {
        out.insert("maxOutputTokens".into(), json!(v));
    }
    out
}

fn strip_stream_function_call_config(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("toolConfig");
        map.remove("tool_config");
    }
}

async fn generate_stream(
    request: ChatRequest,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let url = gemini_url(&request.provider.endpoint, &request.model, true);
    let mut body = build_body(&request, true);
    let provider_label = provider_label(&request);
    let client = crate::ai::providers::build_chat_client()?;
    let mut stripped_arg_stream = false;

    for attempt in 1..=MAX_ATTEMPTS {
        if attempt == 1 {
            debug_log_upstream_request(&provider_label, &url, &body);
        }
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &request.provider.api_key)
            .json(&body)
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
            let emitted = Arc::new(AtomicBool::new(false));
            let flag = emitted.clone();
            let base = on_text_delta.clone();
            let tracked: TextDeltaCallback = Arc::new(move |delta| {
                flag.store(true, Ordering::Relaxed);
                (base)(delta);
            });
            if is_json_response(&resp) {
                let txt = resp.text().await?;
                let parsed = parse_response(&txt)?;
                emit_gemini_response(&parsed, &tracked);
                return Ok(parsed);
            }
            return match consume_gemini_sse(resp, tracked).await {
                Ok(r) => Ok(r),
                Err(e)
                    if should_retry_failed_stream_attempt(
                        &e,
                        attempt,
                        emitted.load(Ordering::Relaxed),
                    ) =>
                {
                    sleep_for_attempt(attempt).await;
                    continue;
                }
                Err(e) => Err(e),
            };
        }

        let txt = match resp.text().await {
            Ok(txt) => txt,
            Err(err) => {
                if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                    sleep_for_attempt(attempt).await;
                    continue;
                }
                return Err(err.into());
            }
        };
        let msg = upstream_error_message(&txt);
        if !stripped_arg_stream && message_rejects_gemini_arg_streaming(&msg) {
            stripped_arg_stream = true;
            strip_stream_function_call_config(&mut body);
            continue;
        }
        if attempt < MAX_ATTEMPTS && should_retry_http_error(status, &msg) {
            sleep_for_attempt(attempt).await;
            continue;
        }
        if status == StatusCode::NOT_FOUND
            || msg.to_ascii_lowercase().contains("streaming is not supported")
        {
            let parsed = generate(request).await?;
            emit_gemini_response(&parsed, &on_text_delta);
            return Ok(parsed);
        }
        return Err(AppError::Upstream(format!(
            "{provider_label} HTTP {status}: {msg}"
        )));
    }
    unreachable!("Gemini stream attempts should return before completing the loop");
}

fn emit_gemini_response(resp: &GenerateResponse, on_text_delta: &TextDeltaCallback) {
    if let Some(t) = resp.thinking_content.as_deref() {
        if !t.is_empty() {
            emit_thinking_deltas(on_text_delta, t);
        }
    }
    if let Some(t) = resp.text.as_deref() {
        if !t.is_empty() {
            (on_text_delta)(StreamDelta::text(t.to_string()));
        }
    }
    for tc in &resp.tool_calls {
        let args = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into());
        emit_tool_arg_deltas(on_text_delta, &tc.id, &tc.name, &args);
    }
}

async fn consume_gemini_sse(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut images = Vec::new();
    let mut usage_acc = TokenUsage::default();
    let mut calls: Vec<GeminiFunctionCallBuilder> = Vec::new();
    let mut sse_debug_emitted = 0u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(stream_read_error)?;
        buffer.extend_from_slice(&chunk);
        while let Some((event_end, sep_len)) = find_sse_event_end(&buffer) {
            let drained: Vec<u8> = buffer.drain(..event_end + sep_len).collect();
            let event = String::from_utf8_lossy(&drained[..event_end]);
            debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
            handle_gemini_sse_event(
                &event,
                &mut text,
                &mut thinking,
                &mut images,
                &mut usage_acc,
                &mut calls,
                &on_text_delta,
            )?;
        }
    }
    if !buffer.is_empty() {
        let event = String::from_utf8_lossy(&buffer);
        debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
        handle_gemini_sse_event(
            &event,
            &mut text,
            &mut thinking,
            &mut images,
            &mut usage_acc,
            &mut calls,
            &on_text_delta,
        )?;
    }

    for call in &mut calls {
        call.finish(&on_text_delta);
    }

    let tool_calls: Vec<crate::ai::chat::ProviderToolCall> = calls
        .into_iter()
        .filter(|c| !c.name.is_empty())
        .map(GeminiFunctionCallBuilder::into_tool_call)
        .collect();
    if text.trim().is_empty()
        && thinking.trim().is_empty()
        && images.is_empty()
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(
            "upstream stream did not contain generated image, text, or tool_calls".into(),
        ));
    }
    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text: if text.trim().is_empty() {
            None
        } else {
            Some(text)
        },
        thinking_content: if thinking.trim().is_empty() {
            None
        } else {
            Some(thinking)
        },
        usage: usage_acc,
        tool_calls,
        response_id: None,
    })
}

fn handle_gemini_sse_event(
    event: &str,
    text: &mut String,
    thinking: &mut String,
    images: &mut Vec<ImageResult>,
    usage_acc: &mut TokenUsage,
    calls: &mut Vec<GeminiFunctionCallBuilder>,
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
    if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
        return Err(AppError::Upstream(msg.to_string()));
    }
    let next_usage = usage(&v);
    if next_usage.prompt_tokens.is_some() {
        usage_acc.prompt_tokens = next_usage.prompt_tokens;
    }
    if next_usage.completion_tokens.is_some() {
        usage_acc.completion_tokens = next_usage.completion_tokens;
    }
    if next_usage.total_tokens.is_some() {
        usage_acc.total_tokens = next_usage.total_tokens;
    }
    if next_usage.cache_read_tokens.is_some() {
        usage_acc.cache_read_tokens = next_usage.cache_read_tokens;
    }
    let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for part in parts {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                if !t.is_empty() {
                    thinking.push_str(t);
                    emit_thinking_deltas(on_text_delta, t);
                }
            }
            continue;
        }
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            if !t.is_empty() {
                text.push_str(t);
                (on_text_delta)(StreamDelta::text(t.to_string()));
            }
        }
        if let Some(image) = image_from_part(part) {
            images.push(image);
        }
        if let Some(fc) = part.get("functionCall").or_else(|| part.get("function_call")) {
            ingest_gemini_function_call(calls, fc, on_text_delta);
        }
    }
    Ok(())
}

fn ingest_gemini_function_call(
    calls: &mut Vec<GeminiFunctionCallBuilder>,
    fc: &Value,
    on_text_delta: &TextDeltaCallback,
) {
    let name = fc
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let idx = if !name.is_empty() {
        calls
            .iter()
            .position(|c| c.name == name && !c.finished)
            .unwrap_or_else(|| {
                let id = format!("gemini-{}", calls.len() + 1);
                calls.push(GeminiFunctionCallBuilder::new(id, name.clone()));
                calls.len() - 1
            })
    } else if let Some(i) = calls.iter().rposition(|c| !c.finished) {
        i
    } else {
        let id = format!("gemini-{}", calls.len() + 1);
        calls.push(GeminiFunctionCallBuilder::new(id, String::new()));
        calls.len() - 1
    };
    calls[idx].apply_chunk(fc, on_text_delta);
}

#[derive(Debug)]
struct GeminiFunctionCallBuilder {
    id: String,
    name: String,
    args: Value,
    started: bool,
    finished: bool,
    identity_emitted: bool,
    need_comma: bool,
    open_string_key: Option<String>,
}

impl GeminiFunctionCallBuilder {
    fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            args: json!({}),
            started: false,
            finished: false,
            identity_emitted: false,
            need_comma: false,
            open_string_key: None,
        }
    }

    fn apply_chunk(&mut self, fc: &Value, cb: &TextDeltaCallback) {
        if let Some(name) = fc.get("name").and_then(Value::as_str) {
            if !name.is_empty() {
                self.name = name.to_string();
            }
        }
        if let Some(args) = fc.get("args") {
            if args.is_object() {
                merge_json_objects(&mut self.args, args);
            }
        }
        let has_partials = fc
            .get("partialArgs")
            .or_else(|| fc.get("partial_args"))
            .and_then(Value::as_array)
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        if has_partials {
            if let Some(arr) = fc
                .get("partialArgs")
                .or_else(|| fc.get("partial_args"))
                .and_then(Value::as_array)
            {
                for partial in arr {
                    self.apply_partial(partial, cb);
                }
            }
        } else if let Some(args) = fc.get("args") {
            if args.is_object() && !self.started {
                let serialized = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
                self.ensure_identity(cb);
                emit_tool_arg_deltas(cb, &self.id, &self.name, &serialized);
                self.started = true;
                self.finished = true;
            }
        }
        let will_continue = fc
            .get("willContinue")
            .or_else(|| fc.get("will_continue"))
            .and_then(Value::as_bool)
            .unwrap_or(has_partials);
        if !will_continue {
            self.finish(cb);
        }
    }

    fn apply_partial(&mut self, partial: &Value, cb: &TextDeltaCallback) {
        let path = partial
            .get("jsonPath")
            .or_else(|| partial.get("json_path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let will_continue = partial
            .get("willContinue")
            .or_else(|| partial.get("will_continue"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(value) = partial_arg_value(partial) else {
            return;
        };
        let segs = parse_json_path(path);
        let append = value.is_string();
        let suffix = apply_path_value(&mut self.args, &segs, value.clone(), append);

        if segs.len() != 1 {
            return;
        }
        let PathSeg::Key(key) = &segs[0] else {
            return;
        };
        self.ensure_identity(cb);
        self.ensure_object_open(cb);
        if let Some(s) = value.as_str() {
            let emit_text = suffix.unwrap_or_else(|| s.to_string());
            self.emit_string_field(cb, key, &emit_text, !will_continue);
        } else if !will_continue {
            self.close_open_string(cb);
            self.emit_comma_if_needed(cb);
            let encoded = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
            self.emit(cb, &format!("\"{}\":{}", json_escape_key(key), encoded));
            self.need_comma = true;
        }
    }

    fn emit_string_field(&mut self, cb: &TextDeltaCallback, key: &str, fragment: &str, close: bool) {
        if self.open_string_key.as_deref() != Some(key) {
            self.close_open_string(cb);
            self.emit_comma_if_needed(cb);
            self.emit(cb, &format!("\"{}\":\"", json_escape_key(key)));
            self.open_string_key = Some(key.to_string());
        }
        if !fragment.is_empty() {
            emit_tool_arg_deltas(cb, &self.id, &self.name, &json_escape_fragment(fragment));
        }
        if close {
            self.close_open_string(cb);
        }
    }

    fn close_open_string(&mut self, cb: &TextDeltaCallback) {
        if self.open_string_key.take().is_some() {
            self.emit(cb, "\"");
            self.need_comma = true;
        }
    }

    fn emit_comma_if_needed(&mut self, cb: &TextDeltaCallback) {
        if self.need_comma {
            self.emit(cb, ",");
            self.need_comma = false;
        }
    }

    fn ensure_object_open(&mut self, cb: &TextDeltaCallback) {
        if !self.started {
            self.emit(cb, "{");
            self.started = true;
        }
    }

    fn ensure_identity(&mut self, cb: &TextDeltaCallback) {
        if self.identity_emitted || self.id.is_empty() || self.name.is_empty() {
            return;
        }
        (cb)(StreamDelta::tool_call(
            self.id.clone(),
            self.name.clone(),
            String::new(),
        ));
        self.identity_emitted = true;
    }

    fn emit(&self, cb: &TextDeltaCallback, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        (cb)(StreamDelta::tool_call(
            self.id.clone(),
            self.name.clone(),
            fragment.to_string(),
        ));
    }

    fn finish(&mut self, cb: &TextDeltaCallback) {
        if self.finished {
            return;
        }
        if self.started {
            self.close_open_string(cb);
            self.emit(cb, "}");
        } else if self.args.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
            self.ensure_identity(cb);
            let serialized = serde_json::to_string(&self.args).unwrap_or_else(|_| "{}".into());
            emit_tool_arg_deltas(cb, &self.id, &self.name, &serialized);
        } else {
            self.ensure_identity(cb);
        }
        self.finished = true;
    }

    fn into_tool_call(self) -> crate::ai::chat::ProviderToolCall {
        crate::ai::chat::ProviderToolCall {
            id: self.id,
            name: self.name,
            arguments: self.args,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSeg {
    Key(String),
    Index(usize),
}

fn parse_json_path(path: &str) -> Vec<PathSeg> {
    let mut s = path.trim();
    if let Some(rest) = s.strip_prefix('$') {
        s = rest;
    }
    if let Some(rest) = s.strip_prefix('.') {
        s = rest;
    }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '.' {
            i += 1;
            continue;
        }
        if chars[i] == '[' {
            i += 1;
            let mut n = String::new();
            while i < chars.len() && chars[i].is_ascii_digit() {
                n.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                i += 1;
            }
            if let Ok(idx) = n.parse::<usize>() {
                out.push(PathSeg::Index(idx));
            }
            continue;
        }
        let mut key = String::new();
        while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
            key.push(chars[i]);
            i += 1;
        }
        if !key.is_empty() {
            out.push(PathSeg::Key(key));
        }
    }
    out
}

fn partial_arg_value(partial: &Value) -> Option<Value> {
    if let Some(s) = partial
        .get("stringValue")
        .or_else(|| partial.get("string_value"))
        .and_then(Value::as_str)
    {
        return Some(Value::String(s.to_string()));
    }
    if let Some(n) = partial
        .get("numberValue")
        .or_else(|| partial.get("number_value"))
    {
        if n.is_number() {
            return Some(n.clone());
        }
    }
    if let Some(b) = partial
        .get("boolValue")
        .or_else(|| partial.get("bool_value"))
        .and_then(Value::as_bool)
    {
        return Some(Value::Bool(b));
    }
    if partial.get("nullValue").is_some() || partial.get("null_value").is_some() {
        return Some(Value::Null);
    }
    None
}

fn apply_path_value(
    root: &mut Value,
    segs: &[PathSeg],
    value: Value,
    append_string: bool,
) -> Option<String> {
    if segs.is_empty() {
        *root = value;
        return None;
    }
    if !root.is_object() && !root.is_array() {
        *root = json!({});
    }
    let mut cur = root;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        match seg {
            PathSeg::Key(k) => {
                if !cur.is_object() {
                    *cur = json!({});
                }
                let obj = cur.as_object_mut().unwrap();
                if last {
                    return write_leaf(
                        obj.entry(k.clone()).or_insert(Value::Null),
                        value,
                        append_string,
                    );
                }
                cur = obj.entry(k.clone()).or_insert_with(|| match segs.get(i + 1) {
                    Some(PathSeg::Index(_)) => json!([]),
                    _ => json!({}),
                });
            }
            PathSeg::Index(idx) => {
                if !cur.is_array() {
                    *cur = json!([]);
                }
                let arr = cur.as_array_mut().unwrap();
                while arr.len() <= *idx {
                    arr.push(Value::Null);
                }
                if last {
                    return write_leaf(&mut arr[*idx], value, append_string);
                }
                if arr[*idx].is_null() {
                    arr[*idx] = match segs.get(i + 1) {
                        Some(PathSeg::Index(_)) => json!([]),
                        _ => json!({}),
                    };
                }
                cur = &mut arr[*idx];
            }
        }
    }
    None
}

fn write_leaf(slot: &mut Value, value: Value, append_string: bool) -> Option<String> {
    if append_string {
        if let (Some(next), Some(prev)) = (value.as_str(), slot.as_str()) {
            if next.starts_with(prev) {
                let suffix = next[prev.len()..].to_string();
                *slot = value;
                return Some(suffix);
            }
            let mut joined = prev.to_string();
            joined.push_str(next);
            *slot = Value::String(joined);
            return Some(next.to_string());
        }
        if let Some(next) = value.as_str() {
            *slot = value.clone();
            return Some(next.to_string());
        }
    }
    *slot = value;
    None
}

fn merge_json_objects(target: &mut Value, src: &Value) {
    let Some(src_obj) = src.as_object() else {
        *target = src.clone();
        return;
    };
    if !target.is_object() {
        *target = json!({});
    }
    let obj = target.as_object_mut().unwrap();
    for (k, v) in src_obj {
        obj.insert(k.clone(), v.clone());
    }
}

fn json_escape_fragment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn json_escape_key(s: &str) -> String {
    json_escape_fragment(s)
}

fn parse_response(txt: &str) -> AppResult<GenerateResponse> {
    if txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }
    let v: Value = serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })?;
    if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
        return Err(AppError::Upstream(msg.to_string()));
    }

    let mut texts = Vec::new();
    let mut thoughts = Vec::new();
    let mut images = Vec::new();
    let mut tool_calls: Vec<crate::ai::chat::ProviderToolCall> = Vec::new();
    let mut counter: u32 = 0;
    if let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    {
        for part in parts {
            if part.get("thought").and_then(Value::as_bool) == Some(true) {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        thoughts.push(trimmed.to_string());
                    }
                }
                continue;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
            if let Some(image) = image_from_part(part) {
                images.push(image);
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let args = fc.get("args").cloned().unwrap_or(Value::Null);
                // Gemini doesn't supply call ids; synthesise stable ones
                // per-response so tool_result can correlate.
                counter += 1;
                tool_calls.push(crate::ai::chat::ProviderToolCall {
                    id: format!("gemini-{counter}"),
                    name,
                    arguments: args,
                });
            }
        }
    }

    if texts.is_empty() && thoughts.is_empty() && images.is_empty() && tool_calls.is_empty() {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text or tool calls. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text: if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n\n"))
        },
        thinking_content: if thoughts.is_empty() {
            None
        } else {
            Some(thoughts.join("\n\n"))
        },
        usage: usage(&v),
        tool_calls,
        response_id: None,
    })
}

fn image_from_part(part: &Value) -> Option<ImageResult> {
    let inline = part.get("inline_data").or_else(|| part.get("inlineData"))?;
    let mime = inline
        .get("mime_type")
        .or_else(|| inline.get("mimeType"))
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let data = inline.get("data").and_then(Value::as_str)?;
    B64.decode(data.as_bytes()).ok().map(|bytes| ImageResult {
        bytes,
        mime: mime.to_string(),
    })
}

fn usage(v: &Value) -> TokenUsage {
    let usage = v.get("usageMetadata").unwrap_or(&Value::Null);
    let cache_read = usage
        .get("cachedContentTokenCount")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0);
    TokenUsage {
        prompt_tokens: usage.get("promptTokenCount").and_then(Value::as_i64),
        completion_tokens: usage.get("candidatesTokenCount").and_then(Value::as_i64),
        total_tokens: usage.get("totalTokenCount").and_then(Value::as_i64),
        last_prompt_tokens: None,
        cache_read_tokens: cache_read,
        cache_write_tokens: None,
    }
}

fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

fn empty_response_details(v: &Value) -> String {
    let mut details = Vec::new();
    if let Some(reason) = v
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
    {
        details.push(format!("finishReason={reason}"));
    }
    if v.pointer("/candidates/0/content/parts").is_none() {
        details.push("missing candidates[0].content.parts".to_string());
    }
    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::ai::chat::ToolDefinition;
    use crate::ai::parameters;

    fn collect_cb() -> (TextDeltaCallback, Arc<Mutex<Vec<StreamDelta>>>) {
        let acc = Arc::new(Mutex::new(Vec::new()));
        let acc2 = acc.clone();
        let cb: TextDeltaCallback = Arc::new(move |d| acc2.lock().unwrap().push(d));
        (cb, acc)
    }

    fn joined_tool_args(deltas: &[StreamDelta]) -> String {
        deltas
            .iter()
            .filter_map(|d| d.tool_call.as_ref().map(|t| t.arguments.as_str()))
            .collect()
    }

    fn sample_request(with_tools: bool) -> ChatRequest {
        ChatRequest {
            provider: crate::ai::chat::ProviderConfig {
                id: "g".into(),
                name: "Gemini".into(),
                sdk: GEMINI_SDK.into(),
                endpoint: "https://generativelanguage.googleapis.com/v1beta".into(),
                api_key: "k".into(),
                context_cache_enabled: false,
            },
            model: "gemini-3.1-flash".into(),
            prompt: "hi".into(),
            attachments: Vec::new(),
            system_prompt: String::new(),
            history: Vec::new(),
            parameters: parameters::factory().build(
                "auto".into(),
                "auto".into(),
                crate::data::settings::ModelParamSettings::default(),
            ),
            tools: if with_tools {
                vec![ToolDefinition {
                    name: "CreateDoc".into(),
                    description: "create".into(),
                    schema: json!({ "type": "object" }),
                }]
            } else {
                Vec::new()
            },
            tool_chain: Vec::new(),
            tool_results: Vec::new(),
            pending_assistant_turn: None,
            previous_response_id: None,
            context_cache_enabled: false,
            context_window: None,
            todo_snapshot: None,
            route_providers: Vec::new(),
        }
    }

    #[test]
    fn stream_url_uses_sse_generate_content() {
        let base = "https://generativelanguage.googleapis.com/v1beta";
        assert_eq!(
            gemini_url(base, "gemini-3.1-flash", false),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash:generateContent"
        );
        assert_eq!(
            gemini_url(base, "gemini-3.1-flash", true),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash:streamGenerateContent?alt=sse"
        );
        let templated = "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent";
        assert!(gemini_url(templated, "gemini-3.1-flash", true)
            .ends_with("models/gemini-3.1-flash:streamGenerateContent?alt=sse"));
        let already = "https://example/models/x:streamGenerateContent?key=k";
        assert_eq!(
            gemini_url(already, "ignored", true),
            "https://example/models/x:streamGenerateContent?key=k&alt=sse"
        );
    }

    #[test]
    fn tool_config_only_when_streaming_function_args() {
        let req = sample_request(true);
        let streamed = build_body(&req, true);
        assert_eq!(
            streamed["toolConfig"]["functionCallingConfig"]["streamFunctionCallArguments"],
            true
        );
        let blocked = build_body(&req, false);
        assert!(blocked.get("toolConfig").is_none());
        let no_tools = build_body(&sample_request(false), true);
        assert!(no_tools.get("toolConfig").is_none());
        assert!(no_tools.get("tools").is_none());
    }

    #[test]
    fn parse_response_keeps_thought_only_turns() {
        let txt = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "先想一步" },
                        { "text": "  回答  " }
                    ]
                }
            }]
        }"#;
        let parsed = parse_response(txt).unwrap();
        assert_eq!(parsed.thinking_content.as_deref(), Some("先想一步"));
        assert_eq!(parsed.text.as_deref(), Some("回答"));

        let thinking_only = r#"{
            "candidates": [{
                "content": { "parts": [{ "thought": true, "text": "only thinking" }] }
            }]
        }"#;
        let parsed = parse_response(thinking_only).unwrap();
        assert_eq!(parsed.thinking_content.as_deref(), Some("only thinking"));
        assert!(parsed.text.is_none());
    }

    #[test]
    fn function_call_builder_appends_incremental_string_fragments() {
        let (cb, acc) = collect_cb();
        let mut call = GeminiFunctionCallBuilder::new("gemini-1".into(), "CreateDoc".into());
        call.apply_chunk(
            &json!({
                "name": "CreateDoc",
                "partialArgs": [{
                    "jsonPath": "$.content",
                    "stringValue": "你好",
                    "willContinue": true
                }],
                "willContinue": true
            }),
            &cb,
        );
        call.apply_chunk(
            &json!({
                "partialArgs": [{
                    "jsonPath": "$.content",
                    "stringValue": "世界",
                    "willContinue": false
                }],
                "willContinue": false
            }),
            &cb,
        );
        assert_eq!(call.args["content"], "你好世界");
        let joined = joined_tool_args(&acc.lock().unwrap());
        let parsed: Value = serde_json::from_str(&joined).unwrap();
        assert_eq!(parsed["content"], "你好世界");
    }

    #[test]
    fn function_call_builder_emits_suffix_for_cumulative_string_values() {
        let (cb, acc) = collect_cb();
        let mut call = GeminiFunctionCallBuilder::new("gemini-1".into(), "CreateDoc".into());
        call.apply_chunk(
            &json!({
                "name": "CreateDoc",
                "partialArgs": [{
                    "jsonPath": "$.content",
                    "stringValue": "你好",
                    "willContinue": true
                }],
                "willContinue": true
            }),
            &cb,
        );
        call.apply_chunk(
            &json!({
                "partialArgs": [{
                    "jsonPath": "$.content",
                    "stringValue": "你好世界",
                    "willContinue": false
                }],
                "willContinue": false
            }),
            &cb,
        );
        assert_eq!(call.args["content"], "你好世界");
        let joined = joined_tool_args(&acc.lock().unwrap());
        let parsed: Value = serde_json::from_str(&joined).unwrap();
        assert_eq!(parsed["content"], "你好世界");
    }

    #[test]
    fn function_call_builder_dumps_buffered_args_object() {
        let (cb, acc) = collect_cb();
        let mut call = GeminiFunctionCallBuilder::new("gemini-1".into(), "CreateDoc".into());
        call.apply_chunk(
            &json!({
                "name": "CreateDoc",
                "args": { "title": "第一章", "content": "正文" }
            }),
            &cb,
        );
        assert_eq!(call.args["title"], "第一章");
        let joined = joined_tool_args(&acc.lock().unwrap());
        let parsed: Value = serde_json::from_str(&joined).unwrap();
        assert_eq!(parsed["title"], "第一章");
        assert_eq!(parsed["content"], "正文");
        assert!(call.finished);
    }
}
