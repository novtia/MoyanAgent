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
    is_json_response, message_rejects_gemini_arg_streaming, should_retry_failed_stream_attempt,
    should_retry_http_error, should_retry_transport, sleep_for_attempt, sse_data_payload,
    stream_read_error, MAX_ATTEMPTS,
};
use crate::ai::providers::{ChatProvider, ProviderFuture, GEMINI_SDK, VERTEX_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

/// How Google generateContent requests authenticate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoogleAuthStyle {
    /// Google AI Studio: always `x-goog-api-key`.
    StudioApiKey,
    /// Vertex generateContent: OAuth/JWT → Bearer; API keys → `x-goog-api-key`.
    VertexAuto,
    /// Vertex Model Garden list: this API rejects API keys, always Bearer.
    AlwaysBearer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GoogleAuthKind {
    ApiKey,
    Bearer,
}

pub(crate) fn google_auth_kind(api_key: &str, style: GoogleAuthStyle) -> GoogleAuthKind {
    match style {
        GoogleAuthStyle::StudioApiKey => GoogleAuthKind::ApiKey,
        GoogleAuthStyle::AlwaysBearer => GoogleAuthKind::Bearer,
        GoogleAuthStyle::VertexAuto => {
            let key = api_key.trim();
            // `ya29.` and `ya29c.` access tokens; JWTs from service accounts.
            if key.starts_with("ya29") || key.starts_with("eyJ") {
                GoogleAuthKind::Bearer
            } else {
                GoogleAuthKind::ApiKey
            }
        }
    }
}

pub(crate) fn apply_google_auth(
    req: reqwest::RequestBuilder,
    api_key: &str,
    style: GoogleAuthStyle,
) -> reqwest::RequestBuilder {
    match google_auth_kind(api_key, style) {
        GoogleAuthKind::Bearer => req.bearer_auth(api_key.trim()),
        GoogleAuthKind::ApiKey => req.header("x-goog-api-key", api_key),
    }
}

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
        Box::pin(async move { generate_google(request, GoogleAuthStyle::StudioApiKey).await })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            generate_google_stream(request, on_text_delta, GoogleAuthStyle::StudioApiKey).await
        })
    }
}

pub(crate) async fn generate_google(
    request: ChatRequest,
    auth: GoogleAuthStyle,
) -> AppResult<GenerateResponse> {
    let url = gemini_url(&request.provider.endpoint, &request.model, false);
    let body = build_body(&request, false);
    let provider_label = provider_label(&request);

    let client = crate::ai::providers::build_chat_client()?;

    let resp = apply_google_auth(
        client.post(&url).header("Content-Type", "application/json"),
        &request.provider.api_key,
        auth,
    )
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

pub(crate) fn gemini_url(endpoint: &str, model: &str, stream: bool) -> String {
    let endpoint = endpoint.trim();
    let method = if stream {
        "streamGenerateContent"
    } else {
        "generateContent"
    };
    let mut url = if endpoint.contains("{model}") {
        replace_gemini_method(&endpoint.replace("{model}", model.trim()), method)
    } else if endpoint.contains(":generateContent") || endpoint.contains(":streamGenerateContent") {
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
                    "parameters": crate::ai::chat::with_declared_property_order(&t.schema),
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
    if let Some(safety) = vertex_safety_settings(request) {
        map.insert("safetySettings".into(), safety);
    }
    body
}

const VERTEX_HARM_CATEGORIES: &[&str] = &[
    "HARM_CATEGORY_HATE_SPEECH",
    "HARM_CATEGORY_DANGEROUS_CONTENT",
    "HARM_CATEGORY_HARASSMENT",
    "HARM_CATEGORY_SEXUALLY_EXPLICIT",
];

fn harm_block_threshold(level: &str) -> Option<&'static str> {
    match level.trim().to_ascii_lowercase().as_str() {
        "off" => Some("OFF"),
        "none" | "block_none" => Some("BLOCK_NONE"),
        "high" | "block_only_high" => Some("BLOCK_ONLY_HIGH"),
        "medium" | "block_medium_and_above" => Some("BLOCK_MEDIUM_AND_ABOVE"),
        "low" | "block_low_and_above" => Some("BLOCK_LOW_AND_ABOVE"),
        _ => None,
    }
}

fn vertex_safety_settings(request: &ChatRequest) -> Option<Value> {
    if crate::ai::providers::normalize_sdk(&request.provider.sdk) != VERTEX_SDK {
        return None;
    }
    let threshold = harm_block_threshold(request.provider.safety_threshold.as_deref()?)?;
    Some(json!(VERTEX_HARM_CATEGORIES
        .iter()
        .map(|category| json!({ "category": category, "threshold": threshold }))
        .collect::<Vec<_>>()))
}

/// Vertex / Gemini 3 last-resort dummy when a `functionCall` was not
/// captured with a real thought signature (e.g. history from another
/// provider). Real signatures must be round-tripped when available.
/// See https://ai.google.dev/gemini-api/docs/thought-signatures
const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

fn extract_thought_signature(part: &Value, fc: Option<&Value>) -> Option<String> {
    let from_value = |v: &Value| {
        v.get("thoughtSignature")
            .or_else(|| v.get("thought_signature"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    from_value(part).or_else(|| fc.and_then(from_value))
}

fn gemini_function_call_part(name: &str, args: &Value, thought_signature: Option<&str>) -> Value {
    let mut part = json!({
        "functionCall": { "name": name, "args": args }
    });
    if let Some(sig) = thought_signature.map(str::trim).filter(|s| !s.is_empty()) {
        part["thoughtSignature"] = json!(sig);
    }
    part
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
    // Gemini 3 requires a thought signature on the first functionCall of
    // each model step. Parallel follow-ups in the same step omit it.
    let mut first_fc = true;
    for tc in &pending.tool_calls {
        let sig = tc
            .thought_signature
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let sig = match sig {
            Some(s) => Some(s),
            None if first_fc => Some(SKIP_THOUGHT_SIGNATURE),
            None => None,
        };
        parts.push(gemini_function_call_part(&tc.name, &tc.arguments, sig));
        first_fc = false;
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
    apply_gemini_thinking_config(&mut out, request);
    out
}

/// Vertex (and current Gemini API) only return thought summaries when
/// `thinkingConfig.includeThoughts` is true. Without it the model still
/// spends thinking tokens, but parts never carry `thought: true`.
fn apply_gemini_thinking_config(out: &mut Map<String, Value>, request: &ChatRequest) {
    let effort = request.parameters.model.resolved_thinking_effort();
    if effort.is_none() && !gemini_supports_thought_summaries(&request.model) {
        return;
    }
    let mut thinking = Map::new();
    thinking.insert("includeThoughts".into(), json!(true));
    if let Some(effort) = effort {
        if gemini_uses_thinking_level(&request.model) {
            thinking.insert(
                "thinkingLevel".into(),
                json!(gemini_thinking_level(&effort)),
            );
        } else {
            thinking.insert(
                "thinkingBudget".into(),
                json!(gemini_thinking_budget(&effort)),
            );
        }
    }
    out.insert("thinkingConfig".into(), Value::Object(thinking));
}

fn gemini_supports_thought_summaries(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    if gemini_image_or_video_model(&m) {
        return false;
    }
    m.contains("gemini-2.5") || m.contains("gemini-3") || m.contains("thinking")
}

fn gemini_uses_thinking_level(model: &str) -> bool {
    model.to_ascii_lowercase().contains("gemini-3")
}

fn gemini_image_or_video_model(model: &str) -> bool {
    model.contains("image") || model.contains("imagen") || model.contains("veo")
}

fn gemini_thinking_level(effort: &str) -> &'static str {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "none" => "MINIMAL",
        "low" => "LOW",
        "medium" => "MEDIUM",
        _ => "HIGH",
    }
}

fn gemini_thinking_budget(effort: &str) -> i64 {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "none" => 0,
        "low" => 2048,
        "medium" => 8192,
        "high" => 24576,
        _ => -1,
    }
}

fn strip_stream_function_call_config(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("toolConfig");
        map.remove("tool_config");
    }
}

pub(crate) async fn generate_google_stream(
    request: ChatRequest,
    on_text_delta: TextDeltaCallback,
    auth: GoogleAuthStyle,
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
        let resp = apply_google_auth(
            client.post(&url).header("Content-Type", "application/json"),
            &request.provider.api_key,
            auth,
        )
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
            || msg
                .to_ascii_lowercase()
                .contains("streaming is not supported")
        {
            let parsed = generate_google(request, auth).await?;
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

/// Streamed content plus the stop diagnostics Gemini reports *only* on chunks
/// that carry no `content.parts` (`finishReason`, `promptFeedback.blockReason`,
/// blocked `safetyRatings`). A safety block looks exactly like an empty stream
/// unless those fields are kept, so accumulate them alongside the content.
#[derive(Default)]
struct GeminiStreamAcc {
    text: String,
    thinking: String,
    images: Vec<ImageResult>,
    usage: TokenUsage,
    calls: Vec<GeminiFunctionCallBuilder>,
    finish_reason: Option<String>,
    finish_message: Option<String>,
    block_reason: Option<String>,
    blocked_categories: Vec<String>,
    saw_candidates: bool,
}

impl GeminiStreamAcc {
    fn record_stop_diagnostics(&mut self, v: &Value) {
        if v.pointer("/candidates/0").is_some() {
            self.saw_candidates = true;
        }
        if let Some(reason) = v
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(reason.to_string());
        }
        if let Some(msg) = v
            .pointer("/candidates/0/finishMessage")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            self.finish_message = Some(msg.to_string());
        }
        if let Some(reason) = v
            .pointer("/promptFeedback/blockReason")
            .and_then(Value::as_str)
        {
            self.block_reason = Some(reason.to_string());
        }
        for path in [
            "/candidates/0/safetyRatings",
            "/promptFeedback/safetyRatings",
        ] {
            let Some(ratings) = v.pointer(path).and_then(Value::as_array) else {
                continue;
            };
            for rating in ratings {
                if rating.get("blocked").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let Some(category) = rating.get("category").and_then(Value::as_str) else {
                    continue;
                };
                if !self.blocked_categories.iter().any(|c| c == category) {
                    self.blocked_categories.push(category.to_string());
                }
            }
        }
    }

    fn empty_stream_details(&self) -> String {
        let mut details = Vec::new();
        if let Some(reason) = &self.finish_reason {
            details.push(format!("finishReason={reason}"));
        }
        if let Some(reason) = &self.block_reason {
            details.push(format!("blockReason={reason}"));
        }
        if !self.blocked_categories.is_empty() {
            details.push(format!("blocked={}", self.blocked_categories.join(", ")));
        }
        if let Some(msg) = &self.finish_message {
            details.push(format!("finishMessage={msg}"));
        }
        if !self.saw_candidates {
            details.push("no candidates[] in any chunk".to_string());
        }
        if details.is_empty() {
            "details: upstream reported no finishReason or blockReason".to_string()
        } else {
            format!("details: {}", details.join("; "))
        }
    }
}

async fn consume_gemini_sse(
    resp: reqwest::Response,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut stream = resp.bytes_stream();
    let mut buffer = Vec::new();
    let mut acc = GeminiStreamAcc::default();
    let mut sse_debug_emitted = 0u32;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(stream_read_error)?;
        buffer.extend_from_slice(&chunk);
        while let Some((event_end, sep_len)) = find_sse_event_end(&buffer) {
            let drained: Vec<u8> = buffer.drain(..event_end + sep_len).collect();
            let event = String::from_utf8_lossy(&drained[..event_end]);
            debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
            handle_gemini_sse_event(&event, &mut acc, &on_text_delta)?;
        }
    }
    if !buffer.is_empty() {
        let event = String::from_utf8_lossy(&buffer);
        debug_log_sse_event(&mut sse_debug_emitted, 12, &event);
        handle_gemini_sse_event(&event, &mut acc, &on_text_delta)?;
    }

    for call in &mut acc.calls {
        call.finish(&on_text_delta);
    }

    let tool_calls: Vec<crate::ai::chat::ProviderToolCall> = std::mem::take(&mut acc.calls)
        .into_iter()
        .filter(|c| !c.name.is_empty())
        .map(GeminiFunctionCallBuilder::into_tool_call)
        .collect();
    if acc.text.trim().is_empty()
        && acc.thinking.trim().is_empty()
        && acc.images.is_empty()
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(format!(
            "upstream stream did not contain generated image, text, or tool_calls. {}",
            acc.empty_stream_details()
        )));
    }
    Ok(GenerateResponse {
        images: acc.images,
        videos: Vec::new(),
        text: if acc.text.trim().is_empty() {
            None
        } else {
            Some(acc.text)
        },
        thinking_content: if acc.thinking.trim().is_empty() {
            None
        } else {
            Some(acc.thinking)
        },
        usage: acc.usage,
        tool_calls,
        response_id: None,
    })
}

fn handle_gemini_sse_event(
    event: &str,
    acc: &mut GeminiStreamAcc,
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
        acc.usage.prompt_tokens = next_usage.prompt_tokens;
    }
    if next_usage.completion_tokens.is_some() {
        acc.usage.completion_tokens = next_usage.completion_tokens;
    }
    if next_usage.total_tokens.is_some() {
        acc.usage.total_tokens = next_usage.total_tokens;
    }
    if next_usage.cache_read_tokens.is_some() {
        acc.usage.cache_read_tokens = next_usage.cache_read_tokens;
    }
    acc.record_stop_diagnostics(&v);
    let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    for part in parts {
        let is_thought = part.get("thought").and_then(Value::as_bool) == Some(true);
        if is_thought {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                if !t.is_empty() {
                    acc.thinking.push_str(t);
                    emit_thinking_deltas(on_text_delta, t);
                }
            }
        } else if let Some(t) = part.get("text").and_then(Value::as_str) {
            if !t.is_empty() {
                acc.text.push_str(t);
                (on_text_delta)(StreamDelta::text(t.to_string()));
            }
        }
        if let Some(image) = image_from_part(part) {
            acc.images.push(image);
        }
        if let Some(fc) = part
            .get("functionCall")
            .or_else(|| part.get("function_call"))
        {
            ingest_gemini_function_call(&mut acc.calls, part, fc, on_text_delta);
        }
    }
    Ok(())
}

fn new_gemini_tool_call_id() -> String {
    format!("gemini-{}", ulid::Ulid::new())
}

fn ingest_gemini_function_call(
    calls: &mut Vec<GeminiFunctionCallBuilder>,
    part: &Value,
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
                calls.push(GeminiFunctionCallBuilder::new(
                    new_gemini_tool_call_id(),
                    name.clone(),
                ));
                calls.len() - 1
            })
    } else if let Some(i) = calls.iter().rposition(|c| !c.finished) {
        i
    } else {
        calls.push(GeminiFunctionCallBuilder::new(
            new_gemini_tool_call_id(),
            String::new(),
        ));
        calls.len() - 1
    };
    calls[idx].apply_chunk(fc, on_text_delta);
    if let Some(sig) = extract_thought_signature(part, Some(fc)) {
        calls[idx].thought_signature = Some(sig);
    }
}

#[derive(Debug)]
struct GeminiFunctionCallBuilder {
    id: String,
    name: String,
    args: Value,
    thought_signature: Option<String>,
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
            thought_signature: None,
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

    fn emit_string_field(
        &mut self,
        cb: &TextDeltaCallback,
        key: &str,
        fragment: &str,
        close: bool,
    ) {
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
        } else if self
            .args
            .as_object()
            .map(|o| !o.is_empty())
            .unwrap_or(false)
        {
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
            thought_signature: self.thought_signature,
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
                cur = obj
                    .entry(k.clone())
                    .or_insert_with(|| match segs.get(i + 1) {
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
    if let Some(parts) = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
    {
        for part in parts {
            let is_thought = part.get("thought").and_then(Value::as_bool) == Some(true);
            if is_thought {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        thoughts.push(trimmed.to_string());
                    }
                }
            } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    texts.push(trimmed.to_string());
                }
            }
            if let Some(image) = image_from_part(part) {
                images.push(image);
            }
            if let Some(fc) = part
                .get("functionCall")
                .or_else(|| part.get("function_call"))
            {
                let name = fc
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let args = fc.get("args").cloned().unwrap_or(Value::Null);
                // Gemini doesn't supply call ids. Synthesise a globally unique
                // one so later agent-loop rounds don't collide with `gemini-1`
                // from a previous round (the live UI keyed cards by that id).
                tool_calls.push(crate::ai::chat::ProviderToolCall {
                    id: new_gemini_tool_call_id(),
                    name,
                    arguments: args,
                    thought_signature: extract_thought_signature(part, Some(fc)),
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
                safety_threshold: None,
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
    fn studio_auth_always_uses_api_key_header() {
        assert_eq!(
            google_auth_kind("ya29.token", GoogleAuthStyle::StudioApiKey),
            GoogleAuthKind::ApiKey
        );
        assert_eq!(
            google_auth_kind("AIzaSyKey", GoogleAuthStyle::StudioApiKey),
            GoogleAuthKind::ApiKey
        );
    }

    #[test]
    fn vertex_auth_uses_bearer_for_oauth_and_jwt() {
        assert_eq!(
            google_auth_kind("ya29.a0Af...", GoogleAuthStyle::VertexAuto),
            GoogleAuthKind::Bearer
        );
        assert_eq!(
            google_auth_kind(
                "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.aaa.sig",
                GoogleAuthStyle::VertexAuto
            ),
            GoogleAuthKind::Bearer
        );
        assert_eq!(
            google_auth_kind("AIzaSyVertexKey", GoogleAuthStyle::VertexAuto),
            GoogleAuthKind::ApiKey
        );
        assert_eq!(
            google_auth_kind("ya29c.token", GoogleAuthStyle::VertexAuto),
            GoogleAuthKind::Bearer
        );
        assert_eq!(
            google_auth_kind("AIzaSyVertexKey", GoogleAuthStyle::AlwaysBearer),
            GoogleAuthKind::Bearer
        );
    }

    #[test]
    fn vertex_templated_url_replaces_model_and_stream_method() {
        let global = "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(global, "gemini-2.5-flash", false),
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
        );
        assert_eq!(
            gemini_url(global, "gemini-2.5-flash", true),
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        let regional = "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(regional, "gemini-2.5-pro", true),
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
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
        let templated =
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent";
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
    fn function_parameters_preserve_property_order() {
        let req = ChatRequest {
            tools: vec![ToolDefinition {
                name: "Edit".into(),
                description: "edit".into(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_string": { "type": "string" },
                        "new_string": { "type": "string" },
                        "replace_all": { "type": "boolean" }
                    }
                }),
            }],
            ..sample_request(false)
        };
        let body = build_body(&req, false);
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["parameters"]["propertyOrdering"],
            json!(["path", "old_string", "new_string", "replace_all"])
        );
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

    #[test]
    fn parse_response_captures_thought_signature_on_function_call_part() {
        let txt = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        {
                            "functionCall": { "name": "Read", "args": { "path": "a.md" } },
                            "thoughtSignature": "sig-on-part"
                        }
                    ]
                }
            }]
        }"#;
        let parsed = parse_response(txt).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "Read");
        assert_eq!(
            parsed.tool_calls[0].thought_signature.as_deref(),
            Some("sig-on-part")
        );
        assert!(parsed.tool_calls[0].id.starts_with("gemini-"));
        assert!(parsed.tool_calls[0].id.len() > "gemini-".len());
    }

    #[test]
    fn parse_response_assigns_unique_tool_ids_across_rounds() {
        let txt = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "functionCall": { "name": "ListFiles", "args": {} } },
                        { "functionCall": { "name": "Read", "args": { "path": "a.md" } } }
                    ]
                }
            }]
        }"#;
        let first = parse_response(txt).unwrap();
        let second = parse_response(txt).unwrap();
        assert_eq!(first.tool_calls.len(), 2);
        assert_ne!(first.tool_calls[0].id, first.tool_calls[1].id);
        assert_ne!(first.tool_calls[0].id, second.tool_calls[0].id);
        assert_ne!(first.tool_calls[1].id, second.tool_calls[1].id);
    }

    #[test]
    fn parse_response_captures_thought_signature_inside_function_call() {
        let txt = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "Read",
                            "args": {},
                            "thoughtSignature": "sig-inside"
                        }
                    }]
                }
            }]
        }"#;
        let parsed = parse_response(txt).unwrap();
        assert_eq!(
            parsed.tool_calls[0].thought_signature.as_deref(),
            Some("sig-inside")
        );
    }

    #[test]
    fn assistant_tool_turn_round_trips_thought_signature() {
        use crate::ai::chat::{PendingAssistantTurn, ProviderToolCall};
        let mut contents = Vec::new();
        let mut first = ProviderToolCall::new("gemini-1", "Read", json!({"path": "a.md"}));
        first.thought_signature = Some("real-sig".into());
        append_gemini_assistant_tool_turn(
            &mut contents,
            &PendingAssistantTurn {
                text: None,
                thinking_content: None,
                tool_calls: vec![first, ProviderToolCall::new("gemini-2", "Grep", json!({}))],
            },
        );
        assert_eq!(contents[0]["parts"][0]["thoughtSignature"], "real-sig");
        assert!(contents[0]["parts"][1].get("thoughtSignature").is_none());
        assert_eq!(contents[0]["parts"][0]["functionCall"]["name"], "Read");
    }

    #[test]
    fn assistant_tool_turn_uses_skip_sentinel_when_signature_missing() {
        use crate::ai::chat::{PendingAssistantTurn, ProviderToolCall};
        let mut contents = Vec::new();
        append_gemini_assistant_tool_turn(
            &mut contents,
            &PendingAssistantTurn {
                text: Some("先读".into()),
                thinking_content: None,
                tool_calls: vec![ProviderToolCall::new("gemini-1", "Read", json!({}))],
            },
        );
        assert_eq!(contents[0]["parts"][0]["text"], "先读");
        assert_eq!(
            contents[0]["parts"][1]["thoughtSignature"],
            SKIP_THOUGHT_SIGNATURE
        );
    }

    #[test]
    fn stream_event_captures_thought_signature_from_part() {
        let (cb, _) = collect_cb();
        let mut acc = GeminiStreamAcc::default();
        handle_gemini_sse_event(
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"Read","args":{"path":"a.md"}},"thoughtSignature":"stream-sig"}]}}]}"#,
            &mut acc,
            &cb,
        )
        .unwrap();
        assert_eq!(acc.calls.len(), 1);
        assert_eq!(
            acc.calls[0].thought_signature.as_deref(),
            Some("stream-sig")
        );
        assert!(acc.calls[0].id.starts_with("gemini-"));
        let first_id = acc.calls[0].id.clone();
        handle_gemini_sse_event(
            r#"data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"Grep","args":{"pattern":"x"}}}]}}]}"#,
            &mut acc,
            &cb,
        )
        .unwrap();
        assert_eq!(acc.calls.len(), 2);
        assert_ne!(first_id, acc.calls[1].id);
        let tc = GeminiFunctionCallBuilder::into_tool_call(acc.calls.remove(0));
        assert_eq!(tc.thought_signature.as_deref(), Some("stream-sig"));
        assert_eq!(tc.name, "Read");
    }

    #[test]
    fn empty_stream_details_surface_safety_block() {
        let (cb, _) = collect_cb();
        let mut acc = GeminiStreamAcc::default();
        handle_gemini_sse_event(
            r#"data: {"candidates":[{"finishReason":"SAFETY","safetyRatings":[{"category":"HARM_CATEGORY_SEXUALLY_EXPLICIT","probability":"HIGH","blocked":true},{"category":"HARM_CATEGORY_HATE_SPEECH","probability":"LOW"}]}]}"#,
            &mut acc,
            &cb,
        )
        .unwrap();
        let details = acc.empty_stream_details();
        assert!(details.contains("finishReason=SAFETY"), "{details}");
        assert!(
            details.contains("blocked=HARM_CATEGORY_SEXUALLY_EXPLICIT"),
            "{details}"
        );
        assert!(!details.contains("HARM_CATEGORY_HATE_SPEECH"), "{details}");
    }

    #[test]
    fn empty_stream_details_surface_prompt_block() {
        let (cb, _) = collect_cb();
        let mut acc = GeminiStreamAcc::default();
        handle_gemini_sse_event(
            r#"data: {"promptFeedback":{"blockReason":"PROHIBITED_CONTENT"},"usageMetadata":{"promptTokenCount":12}}"#,
            &mut acc,
            &cb,
        )
        .unwrap();
        let details = acc.empty_stream_details();
        assert!(
            details.contains("blockReason=PROHIBITED_CONTENT"),
            "{details}"
        );
        assert!(
            details.contains("no candidates[] in any chunk"),
            "{details}"
        );
        assert_eq!(acc.usage.prompt_tokens, Some(12));
    }

    #[test]
    fn empty_stream_details_report_absence_of_reason() {
        let acc = GeminiStreamAcc::default();
        let details = acc.empty_stream_details();
        assert!(
            details.contains("no candidates[] in any chunk"),
            "{details}"
        );
    }

    fn enable_thinking(req: &mut ChatRequest, effort: &str) {
        req.parameters.model.thinking_enabled = Some(true);
        req.parameters.model.thinking_effort = Some(effort.into());
    }

    #[test]
    fn thinking_models_request_thought_summaries_by_default() {
        let req = sample_request(false);
        let body = build_body(&req, false);
        let cfg = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(cfg["includeThoughts"], true);
        assert!(cfg.get("thinkingLevel").is_none());
        assert!(cfg.get("thinkingBudget").is_none());
    }

    #[test]
    fn gemini_3_thinking_sends_level_and_include_thoughts() {
        let mut req = sample_request(false);
        req.model = "gemini-3-flash-preview".into();
        enable_thinking(&mut req, "high");
        let cfg = &build_body(&req, true)["generationConfig"]["thinkingConfig"];
        assert_eq!(cfg["includeThoughts"], true);
        assert_eq!(cfg["thinkingLevel"], "HIGH");
        assert!(cfg.get("thinkingBudget").is_none());

        enable_thinking(&mut req, "low");
        let cfg = &build_body(&req, false)["generationConfig"]["thinkingConfig"];
        assert_eq!(cfg["thinkingLevel"], "LOW");
    }

    #[test]
    fn gemini_2_5_thinking_sends_budget_and_include_thoughts() {
        let mut req = sample_request(false);
        req.provider.sdk = VERTEX_SDK.into();
        req.model = "gemini-2.5-pro".into();
        enable_thinking(&mut req, "medium");
        let cfg = &build_body(&req, true)["generationConfig"]["thinkingConfig"];
        assert_eq!(cfg["includeThoughts"], true);
        assert_eq!(cfg["thinkingBudget"], 8192);
        assert!(cfg.get("thinkingLevel").is_none());
    }

    #[test]
    fn image_models_do_not_request_thought_summaries() {
        let mut req = sample_request(false);
        req.model = "gemini-2.5-flash-image".into();
        let body = build_body(&req, false);
        assert!(body.pointer("/generationConfig/thinkingConfig").is_none());
    }

    #[test]
    fn vertex_safety_settings_use_chosen_threshold() {
        let mut req = sample_request(false);
        req.provider.sdk = VERTEX_SDK.into();
        req.provider.safety_threshold = Some("none".into());
        let body = build_body(&req, false);
        let arr = body["safetySettings"].as_array().expect("safetySettings");
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0]["category"], "HARM_CATEGORY_HATE_SPEECH");
        assert_eq!(arr[0]["threshold"], "BLOCK_NONE");
        assert!(arr.iter().all(|p| p["threshold"] == "BLOCK_NONE"));

        req.provider.safety_threshold = Some("off".into());
        let off = build_body(&req, false);
        assert_eq!(off["safetySettings"][0]["threshold"], "OFF");

        req.provider.safety_threshold = Some("low".into());
        let strict = build_body(&req, false);
        assert_eq!(
            strict["safetySettings"][0]["threshold"],
            "BLOCK_LOW_AND_ABOVE"
        );
    }

    #[test]
    fn vertex_default_and_studio_omit_safety_settings() {
        let mut req = sample_request(false);
        req.provider.sdk = VERTEX_SDK.into();
        req.provider.safety_threshold = None;
        assert!(build_body(&req, false).get("safetySettings").is_none());

        req.provider.sdk = GEMINI_SDK.into();
        req.provider.safety_threshold = Some("none".into());
        assert!(build_body(&req, false).get("safetySettings").is_none());
    }
}
