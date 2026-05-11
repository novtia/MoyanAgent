//! Native xAI Grok Image HTTP API (`https://api.x.ai/v1/images/*`), not OpenAI-compatible chat.
//! See https://docs.x.ai/developers/model-capabilities/images

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, ImageResult};
use crate::ai::providers::{ChatProvider, ProviderFuture, GROK_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
const MAX_ATTEMPTS: usize = 3;
const MAX_EDIT_IMAGES: usize = 3;

pub struct GrokProvider;

impl GrokProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for GrokProvider {
    fn sdk(&self) -> &'static str {
        GROK_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let prompt = build_prompt(&request);
    if prompt.trim().is_empty() {
        return Err(AppError::Config(
            "Grok image requires a non-empty prompt".into(),
        ));
    }

    let (gen_url, edit_url) = resolve_image_urls(&request.provider.endpoint)?;
    let url = if request.attachments.is_empty() {
        gen_url
    } else {
        edit_url
    };

    let body = if request.attachments.is_empty() {
        build_generations_body(&request, &prompt)
    } else {
        build_edits_body(&request, &prompt)?
    };

    let provider_label = provider_label(&request);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let txt = post_with_retries(
        &client,
        &request.provider.api_key,
        &url,
        &body,
        &provider_label,
    )
    .await?;
    parse_and_fetch_images(&client, &txt, &provider_label).await
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

/// `endpoint` may be `https://api.x.ai/v1/images/generations`, a base `https://api.x.ai/v1`, or the edits URL.
fn resolve_image_urls(endpoint: &str) -> AppResult<(String, String)> {
    let e = endpoint.trim().trim_end_matches('/');
    if e.is_empty() {
        return Err(AppError::Config(
            "Grok provider requires endpoint (e.g. https://api.x.ai/v1/images/generations)".into(),
        ));
    }

    if let Some(base) = e.strip_suffix("/images/generations") {
        let edit = format!("{}/images/edits", base);
        return Ok((e.to_string(), edit));
    }
    if let Some(base) = e.strip_suffix("/images/edits") {
        return Ok((format!("{}/images/generations", base), e.to_string()));
    }

    // Base path e.g. https://api.x.ai/v1
    Ok((
        format!("{}/images/generations", e),
        format!("{}/images/edits", e),
    ))
}

fn build_prompt(request: &ChatRequest) -> String {
    let mut parts: Vec<String> = Vec::new();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        parts.push(sys.to_string());
    }
    for turn in &request.history {
        let text = turn.text.as_deref().unwrap_or("").trim();
        if !text.is_empty() {
            parts.push(format!("{}: {}", turn.role, text));
        }
    }
    let p = request.prompt.trim();
    if !p.is_empty() {
        parts.push(p.to_string());
    }
    parts.join("\n\n")
}

fn build_generations_body(request: &ChatRequest, prompt: &str) -> Value {
    let mut body = json!({
        "model": request.model,
        "prompt": prompt,
    });
    if let Some(map) = body.as_object_mut() {
        if request.parameters.aspect_ratio != "auto" {
            map.insert(
                "aspect_ratio".into(),
                Value::String(request.parameters.aspect_ratio.clone()),
            );
        }
    }
    body
}

fn build_edits_body(request: &ChatRequest, prompt: &str) -> AppResult<Value> {
    let n = request.attachments.len();
    if n > MAX_EDIT_IMAGES {
        return Err(AppError::Config(format!(
            "Grok image edit accepts at most {} source images (got {})",
            MAX_EDIT_IMAGES, n
        )));
    }

    let mut body = json!({
        "model": request.model,
        "prompt": prompt,
    });
    let map = body.as_object_mut().unwrap();

    if request.parameters.aspect_ratio != "auto" {
        map.insert(
            "aspect_ratio".into(),
            Value::String(request.parameters.aspect_ratio.clone()),
        );
    }

    if n == 1 {
        map.insert(
            "image".into(),
            json!({
                "type": "image_url",
                "url": data_url(&request.attachments[0]),
            }),
        );
    } else {
        let images: Vec<Value> = request
            .attachments
            .iter()
            .map(|a| {
                json!({
                    "type": "image_url",
                    "url": data_url(a),
                })
            })
            .collect();
        map.insert("images".into(), Value::Array(images));
    }

    Ok(body)
}

fn data_url(attachment: &AttachmentBytes) -> String {
    let b64 = B64.encode(&attachment.bytes);
    format!("data:{};base64,{}", attachment.mime, b64)
}

async fn post_with_retries(
    client: &reqwest::Client,
    api_key: &str,
    url: &str,
    body: &Value,
    provider_label: &str,
) -> AppResult<String> {
    for attempt in 1..=MAX_ATTEMPTS {
        let resp = client
            .post(url)
            .bearer_auth(api_key)
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
        Ok(v) => xai_error_body_to_message(&v).unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

/// xAI image APIs often return `{ "code": "...", "error": "human-readable..." }` on failure.
fn xai_error_body_to_message(v: &Value) -> Option<String> {
    if let Some(msg) = v.get("error").and_then(Value::as_str) {
        let msg = msg.trim();
        if msg.is_empty() {
            return None;
        }
        let code = v
            .get("code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty());
        return Some(match code {
            Some(c) if c != msg => format!("{} ({})", msg, c),
            _ => msg.to_string(),
        });
    }
    v.pointer("/error/message")
        .or_else(|| v.pointer("/error/type"))
        .or_else(|| v.pointer("/message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

async fn parse_and_fetch_images(
    client: &reqwest::Client,
    txt: &str,
    provider_label: &str,
) -> AppResult<GenerateResponse> {
    if txt.trim().is_empty() {
        return Err(AppError::Upstream(
            "xAI returned an empty response body".into(),
        ));
    }
    let v: Value = serde_json::from_str(txt).map_err(|e| {
        AppError::Upstream(format!("invalid JSON from xAI image API: {e}; body: {txt}"))
    })?;

    if let Some(msg) = top_level_error(&v) {
        return Err(AppError::Upstream(msg));
    }

    let mut urls: Vec<String> = Vec::new();
    collect_image_urls(&v, &mut urls);

    if urls.is_empty() {
        let mut from_b64: Vec<ImageResult> = Vec::new();
        collect_b64_images(&v, &mut from_b64);
        if !from_b64.is_empty() {
            return Ok(GenerateResponse {
                images: from_b64,
                text: None,
                thinking_content: None,
                usage: TokenUsage::default(),
                tool_calls: Vec::new(),
            });
        }
        return Err(AppError::Upstream(format!(
            "{}: xAI image response had no image URL or base64 payload: {}",
            provider_label,
            txt.chars().take(500).collect::<String>()
        )));
    }

    let mut images: Vec<ImageResult> = Vec::new();
    for u in urls {
        if let Some(img) = parse_data_url(&u) {
            images.push(img);
            continue;
        }
        if u.starts_with("http://") || u.starts_with("https://") {
            let img = fetch_remote_image(client, &u, provider_label).await?;
            images.push(img);
        }
    }

    if images.is_empty() {
        return Err(AppError::Upstream(format!(
            "{}: could not decode or download any image from xAI response",
            provider_label
        )));
    }

    Ok(GenerateResponse {
        images,
        text: None,
        thinking_content: None,
        usage: TokenUsage::default(),
        tool_calls: Vec::new(),
    })
}

fn top_level_error(v: &Value) -> Option<String> {
    xai_error_body_to_message(v)
}

fn collect_image_urls(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            if s.starts_with("http://") || s.starts_with("https://") {
                out.push(s.clone());
            }
        }
        Value::Array(items) => {
            for it in items {
                collect_image_urls(it, out);
            }
        }
        Value::Object(map) => {
            if let Some(s) = map.get("url").and_then(Value::as_str) {
                if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") {
                    out.push(s.to_string());
                }
            }
            for key in ["data", "images", "output", "result", "results"] {
                if let Some(child) = map.get(key) {
                    collect_image_urls(child, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_b64_images(v: &Value, out: &mut Vec<ImageResult>) {
    match v {
        Value::Object(map) => {
            if let Some(b64) = map
                .get("b64_json")
                .or_else(|| map.get("base64"))
                .and_then(Value::as_str)
            {
                let mime = map
                    .get("mime_type")
                    .or_else(|| map.get("mimeType"))
                    .and_then(Value::as_str);
                if let Ok(bytes) = B64.decode(b64.as_bytes()) {
                    out.push(ImageResult {
                        bytes,
                        mime: mime.unwrap_or("image/png").to_string(),
                    });
                }
            }
            for key in ["data", "images", "output"] {
                if let Some(child) = map.get(key) {
                    collect_b64_images(child, out);
                }
            }
        }
        Value::Array(items) => {
            for it in items {
                collect_b64_images(it, out);
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
    B64.decode(payload.as_bytes())
        .ok()
        .map(|bytes| ImageResult { bytes, mime })
}

async fn fetch_remote_image(
    client: &reqwest::Client,
    url: &str,
    provider_label: &str,
) -> AppResult<ImageResult> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let t = resp.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "{}: failed to download image URL HTTP {}: {}",
            provider_label, status, t
        )));
    }
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .trim()
        .to_string();
    let bytes = resp.bytes().await?.to_vec();
    Ok(ImageResult { bytes, mime })
}
