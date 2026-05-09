use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

pub struct ImageResult {
    pub bytes: Vec<u8>,
    pub mime: String,
}

pub struct GenerateResponse {
    pub images: Vec<ImageResult>,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AttachmentBytes {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// One prior turn of the conversation, fed back to the model as context.
/// `role` is "user" or "assistant". For user turns, `images` should typically be
/// the input/edited references; for assistant turns, the previously generated outputs.
#[derive(Debug, Clone)]
pub struct HistoryTurn {
    pub role: String,
    pub text: Option<String>,
    pub images: Vec<AttachmentBytes>,
}

pub struct GenerateOptions {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub prompt: String,
    pub attachments: Vec<AttachmentBytes>,
    pub aspect_ratio: String,
    pub image_size: String,
    pub system_prompt: String,
    pub history: Vec<HistoryTurn>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
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
    for a in &turn.images {
        arr.push(json!({
            "type": "image_url",
            "image_url": { "url": data_url(a) }
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

pub async fn generate(opts: GenerateOptions) -> AppResult<GenerateResponse> {
    let user_content: Value = if opts.attachments.is_empty() {
        Value::String(opts.prompt.clone())
    } else {
        let mut arr: Vec<Value> = Vec::with_capacity(opts.attachments.len() + 1);
        arr.push(json!({"type":"text","text":opts.prompt}));
        for a in &opts.attachments {
            arr.push(json!({
                "type":"image_url",
                "image_url": { "url": data_url(a) }
            }));
        }
        Value::Array(arr)
    };

    let mut messages: Vec<Value> = Vec::new();
    let sys = opts.system_prompt.trim();
    if !sys.is_empty() {
        messages.push(json!({ "role": "system", "content": sys }));
    }
    for turn in &opts.history {
        if let Some(m) = history_turn_to_message(turn) {
            messages.push(m);
        }
    }
    messages.push(json!({ "role": "user", "content": user_content }));

    let mut body = json!({
        "model": opts.model,
        "modalities": requested_modalities(&opts.model),
        "messages": messages,
    });
    {
        let map = body.as_object_mut().unwrap();
        if let Some(v) = opts.temperature {
            map.insert("temperature".into(), json!(v));
        }
        if let Some(v) = opts.top_p {
            map.insert("top_p".into(), json!(v));
        }
        if let Some(v) = opts.max_tokens {
            map.insert("max_tokens".into(), json!(v));
        }
        if let Some(v) = opts.frequency_penalty {
            map.insert("frequency_penalty".into(), json!(v));
        }
        if let Some(v) = opts.presence_penalty {
            map.insert("presence_penalty".into(), json!(v));
        }
    }
    let mut image_config = serde_json::Map::new();
    if opts.aspect_ratio != "auto" {
        image_config.insert(
            "aspect_ratio".into(),
            Value::String(opts.aspect_ratio.clone()),
        );
    }
    if opts.image_size != "auto" {
        image_config.insert("image_size".into(), Value::String(opts.image_size.clone()));
    }
    if !image_config.is_empty() {
        body.as_object_mut()
            .unwrap()
            .insert("image_config".into(), Value::Object(image_config));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(360))
        .build()?;

    const MAX_ATTEMPTS: usize = 3;
    let mut final_txt = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        let resp = client
            .post(&opts.endpoint)
            .bearer_auth(&opts.api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(err) => {
                if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                    let backoff_ms = 500u64 * (1u64 << (attempt - 1));
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                return Err(err.into());
            }
        };

        let status = resp.status();
        let txt = resp.text().await?;
        final_txt = txt.clone();
        if status.is_success() {
            break;
        }

        let msg = match serde_json::from_str::<Value>(&txt) {
            Ok(v) => v
                .pointer("/error/message")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| txt.clone()),
            Err(_) => txt.clone(),
        };
        if attempt < MAX_ATTEMPTS && is_retryable_status(status) {
            let backoff_ms = 500u64 * (1u64 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            continue;
        }
        return Err(AppError::Upstream(format!("HTTP {}: {}", status, msg)));
    }

    if final_txt.is_empty() {
        return Err(AppError::Upstream("上游返回为空响应".into()));
    }

    let v: Value = parse_response_json(&final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }
    let images = extract_images(&v);
    let text = extract_text(&v);
    if images.is_empty() && text.as_deref().map(|s| s.is_empty()).unwrap_or(true) {
        return Err(AppError::Upstream(format!(
            "上游返回为空：既没有图像也没有文本{}",
            empty_response_details(&v)
        )));
    }
    Ok(GenerateResponse { images, text })
}

fn parse_response_json(txt: &str) -> AppResult<Value> {
    serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "上游返回的 JSON 无法解析（{}，body_bytes={}）。这通常是请求超时、连接被截断或上游返回了非 JSON 内容；请重试，或降低 image_size/换更快的模型。",
            err,
            txt.len()
        ))
    })
}

fn top_level_error_message(v: &Value) -> Option<String> {
    let error = v.get("error")?;
    let message = error
        .get("message")
        .and_then(|x| x.as_str())
        .unwrap_or("上游返回错误");
    let code = error
        .get("code")
        .map(|x| {
            x.as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| x.to_string())
        })
        .filter(|s| !s.trim().is_empty() && s != "null");

    Some(match code {
        Some(code) => format!("上游错误 {}: {}", code, message),
        None => format!("上游错误: {}", message),
    })
}

fn requested_modalities(model: &str) -> Value {
    if is_image_only_model(model) {
        json!(["image"])
    } else {
        json!(["image", "text"])
    }
}

fn is_image_only_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.starts_with("black-forest-labs/")
        || m.starts_with("sourceful/")
        || m.starts_with("recraft/")
}

fn extract_text(v: &Value) -> Option<String> {
    let msg = v.pointer("/choices/0/message")?;

    if let Some(s) = msg.get("content").and_then(|x| x.as_str()) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(arr) = msg.get("content").and_then(|x| x.as_array()) {
        let mut parts: Vec<String> = Vec::new();
        for it in arr {
            if let Some(s) = it.get("text").and_then(|x| x.as_str()) {
                if !s.trim().is_empty() {
                    parts.push(s.trim().to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }

    if let Some(s) = msg.get("reasoning").and_then(|x| x.as_str()) {
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
    B64.decode(payload.as_bytes()).ok().map(|bytes| ImageResult {
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
    .find_map(|x| x.as_str())
}

fn image_from_value(value: &Value) -> Option<ImageResult> {
    if let Some(u) = image_url_from_value(value) {
        if let Some(r) = parse_data_url(u) {
            return Some(r);
        }
    }
    value
        .get("b64_json")
        .and_then(|x| x.as_str())
        .and_then(parse_b64_image)
}

fn extract_images(v: &Value) -> Vec<ImageResult> {
    let mut out = Vec::new();
    let msg = match v.pointer("/choices/0/message") {
        Some(x) => x,
        None => return out,
    };

    if let Some(arr) = msg.get("images").and_then(|x| x.as_array()) {
        for it in arr {
            if let Some(r) = image_from_value(it) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(arr) = msg.get("content").and_then(|x| x.as_array()) {
        for part in arr {
            if let Some(r) = image_from_value(part) {
                out.push(r);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    if let Some(s) = msg.get("content").and_then(|x| x.as_str()) {
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
        .map(|s| s.to_string())
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
    push_response_detail_at(&mut details, v, "/choices/0/error/code", "choice_error_code");
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
        format!("（{}）", details.join("，"))
    }
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OrError {
    message: Option<String>,
    code: Option<i64>,
}
