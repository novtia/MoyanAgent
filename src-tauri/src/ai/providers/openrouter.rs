use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn, ImageResult};
use crate::ai::providers::{ChatProvider, ProviderFuture, OPENROUTER_SDK};
use crate::ai::tokens;
use crate::error::{AppError, AppResult};

pub struct OpenRouterProvider;

impl OpenRouterProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for OpenRouterProvider {
    fn sdk(&self) -> &'static str {
        OPENROUTER_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let mut body = build_request_body(&request);
    let provider_label = provider_label(&request);

    const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    const MAX_ATTEMPTS: usize = 3;
    let mut modality_stage: u8 = 0;

    let final_txt = 'modalities: loop {
        apply_modalities_stage(&mut body, &request.model, modality_stage);

        for attempt in 1..=MAX_ATTEMPTS {
            let resp = client
                .post(&request.provider.endpoint)
                .bearer_auth(&request.provider.api_key)
                .header("Content-Type", "application/json")
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
            let txt = resp.text().await?;
            if status.is_success() {
                break 'modalities txt;
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
        unreachable!("HTTP attempts should return or branch before completing the inner loop");
    };

    if final_txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }

    let v: Value = parse_response_json(&final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let images = extract_images(&v);
    let text = extract_text(&v);
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

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

fn build_request_body(request: &ChatRequest) -> Value {
    let user_content: Value = if request.attachments.is_empty() {
        Value::String(request.prompt.clone())
    } else {
        let mut arr: Vec<Value> = Vec::with_capacity(request.attachments.len() + 1);
        arr.push(json!({"type":"text","text":request.prompt}));
        for attachment in &request.attachments {
            arr.push(json!({
                "type":"image_url",
                "image_url": { "url": data_url(attachment) }
            }));
        }
        Value::Array(arr)
    };

    let mut messages: Vec<Value> = Vec::new();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    for turn in &request.history {
        if let Some(message) = history_turn_to_message(turn) {
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
    if let Some(image_config) = request.parameters.image_config() {
        map.insert("image_config".into(), image_config);
    }
    body
}

fn data_url(att: &AttachmentBytes) -> String {
    format!("data:{};base64,{}", att.mime, B64.encode(&att.bytes))
}

fn history_turn_to_message(turn: &HistoryTurn) -> Option<Value> {
    let role = turn.role.trim();
    if role.is_empty() {
        return None;
    }

    let text = turn
        .text
        .as_deref()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if turn.images.is_empty() {
        if text.is_empty() {
            return None;
        }
        return Some(json!({ "role": role, "content": text }));
    }

    let mut arr: Vec<Value> = Vec::with_capacity(turn.images.len() + 1);
    if !text.is_empty() {
        arr.push(json!({ "type": "text", "text": text }));
    }
    for image in &turn.images {
        arr.push(json!({
            "type": "image_url",
            "image_url": { "url": data_url(image) }
        }));
    }
    Some(json!({ "role": role, "content": Value::Array(arr) }))
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

fn parse_response_json(txt: &str) -> AppResult<Value> {
    serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })
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

fn top_level_error_message(v: &Value) -> Option<String> {
    let error = v.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown upstream error");
    let code = error
        .get("code")
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

fn requested_modalities(model: &str) -> Value {
    if is_image_only_model(model) {
        json!(["image"])
    } else {
        json!(["image", "text"])
    }
}

fn apply_modalities_stage(body: &mut Value, model: &str, stage: u8) {
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

fn extract_text(v: &Value) -> Option<String> {
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

fn parse_b64_image(payload: &str) -> Option<ImageResult> {
    B64.decode(payload.as_bytes())
        .ok()
        .map(|bytes| ImageResult {
            bytes,
            mime: "image/png".to_string(),
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
    value
        .get("b64_json")
        .and_then(Value::as_str)
        .and_then(parse_b64_image)
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
    out
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

    if v.pointer("/choices/0/message").is_none() {
        details.push("missing choices[0].message".to_string());
    }

    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}
