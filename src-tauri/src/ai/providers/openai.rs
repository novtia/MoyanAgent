use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ImageResult};
use crate::ai::parameters::GenerationParameters;
use crate::ai::providers::{
    ChatProvider, ProviderFuture, OPENAI_RESPONSES_SDK, OPENAI_SDK,
};
use crate::ai::{tokens, tokens::TokenUsage};
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
const MAX_ATTEMPTS: usize = 3;

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

async fn generate_responses(request: ChatRequest) -> AppResult<GenerateResponse> {
    let body = build_responses_body(&request);
    let provider_label = provider_label(&request);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let final_txt = post_with_retries(&client, &request, &body, &provider_label).await?;
    parse_responses_response(&final_txt)
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

async fn post_with_retries(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
) -> AppResult<String> {
    for attempt in 1..=MAX_ATTEMPTS {
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

    let mut body = json!({
        "model": request.model,
        "messages": messages,
    });

    let map = body.as_object_mut().unwrap();
    request.parameters.apply_model_params(map);
    if is_openrouter_endpoint(&request.provider.endpoint) {
        if let Some(image_config) = request.parameters.image_config() {
            map.insert("image_config".into(), image_config);
        }
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

    Some(json!({ "role": role, "content": content }))
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
    apply_responses_params(map, &request.parameters);
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

fn apply_responses_params(body: &mut Map<String, Value>, params: &GenerationParameters) {
    if let Some(v) = params.model.temperature {
        body.insert("temperature".into(), json!(v));
    }
    if let Some(v) = params.model.top_p {
        body.insert("top_p".into(), json!(v));
    }
    if let Some(v) = params.model.max_tokens {
        body.insert("max_output_tokens".into(), json!(v));
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
    if images.is_empty() && text.as_deref().map(str::is_empty).unwrap_or(true) {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image or text. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        text,
        usage: tokens::extract_usage(&v),
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
    if images.is_empty() && text.as_deref().map(str::is_empty).unwrap_or(true) {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image or text. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        text,
        usage: tokens::extract_usage(&v),
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

    if let Some(s) = msg.get("reasoning").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
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
