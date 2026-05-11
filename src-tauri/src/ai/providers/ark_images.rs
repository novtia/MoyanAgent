//! 豆包（火山引擎方舟）图片生成 API — `POST …/api/v3/images/generations`（Seedream 等）。
//! 此类模型不能使用 `chat/completions`；见 https://www.volcengine.com/docs/82379/2105966

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, ImageResult};
use crate::ai::parameters::GenerationParameters;
use crate::ai::providers::{ChatProvider, ProviderFuture, ARK_IMAGES_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
const MAX_ATTEMPTS: usize = 3;
const MAX_REF_IMAGES: usize = 10;

pub struct ArkImagesProvider;

impl ArkImagesProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for ArkImagesProvider {
    fn sdk(&self) -> &'static str {
        ARK_IMAGES_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let prompt = build_prompt(&request);
    if prompt.trim().is_empty() {
        return Err(AppError::Config("豆包生图需要非空的提示词".into()));
    }

    let n = request.attachments.len();
    if n > MAX_REF_IMAGES {
        return Err(AppError::Config(format!(
            "豆包 Seedream 最多接受 {} 张参考图（当前 {} 张）",
            MAX_REF_IMAGES, n
        )));
    }

    let url = resolve_generations_url(&request.provider.endpoint);
    let body = build_body(&request, &prompt);
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
    parse_response(&client, &txt, &provider_label).await
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

/// Accepts full `…/images/generations` URL, or `…/chat/completions` (rewritten), or `…/api/v3` base.
fn resolve_generations_url(endpoint: &str) -> String {
    let e = endpoint.trim().trim_end_matches('/');
    if e.is_empty() {
        return String::new();
    }
    if e.ends_with("/chat/completions") {
        if let Some(base) = e.strip_suffix("/chat/completions") {
            return format!("{}/images/generations", base);
        }
    }
    if e.ends_with("/images/generations") {
        return e.to_string();
    }
    format!("{}/images/generations", e)
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
    let mut out = parts.join("\n\n");
    let ar = request.parameters.aspect_ratio.trim();
    if !ar.is_empty() && ar != "auto" {
        out.push_str("\n\n");
        out.push_str(&format!("（画面比例：{}）", ar));
    }
    out
}

fn ark_size(parameters: &GenerationParameters) -> String {
    let s = parameters.image_size.trim();
    if s.is_empty() || s == "auto" {
        return "2K".to_string();
    }
    let upper = s.to_ascii_uppercase();
    if matches!(upper.as_str(), "1K" | "2K" | "3K" | "4K") {
        return upper;
    }
    s.to_string()
}

fn build_body(request: &ChatRequest, prompt: &str) -> Value {
    let mut body = json!({
        "model": request.model,
        "prompt": prompt,
        "size": ark_size(&request.parameters),
        "response_format": "url",
        "watermark": false,
        "sequential_image_generation": "disabled",
        "stream": false,
    });
    let map = body.as_object_mut().unwrap();

    if !request.attachments.is_empty() {
        if request.attachments.len() == 1 {
            map.insert(
                "image".into(),
                Value::String(data_url(&request.attachments[0])),
            );
        } else {
            let imgs: Vec<Value> = request
                .attachments
                .iter()
                .map(|a| Value::String(data_url(a)))
                .collect();
            map.insert("image".into(), Value::Array(imgs));
        }
    }

    body
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
    if url.trim().is_empty() {
        return Err(AppError::Config(
            "豆包生图需要填写 Endpoint（例如 https://ark.cn-beijing.volces.com/api/v3/images/generations）".into(),
        ));
    }

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
        Ok(v) => ark_error_body_to_message(&v).unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

fn ark_error_body_to_message(v: &Value) -> Option<String> {
    if let Some(s) = v.get("error").and_then(Value::as_str) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
        let msg = msg.trim();
        if !msg.is_empty() {
            let code = v
                .pointer("/error/code")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|c| !c.is_empty());
            return Some(match code {
                Some(c) if c != msg => format!("{} ({})", msg, c),
                _ => msg.to_string(),
            });
        }
    }
    v.get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn parse_response(
    client: &reqwest::Client,
    txt: &str,
    provider_label: &str,
) -> AppResult<GenerateResponse> {
    if txt.trim().is_empty() {
        return Err(AppError::Upstream("豆包生图接口返回了空响应".into()));
    }
    let v: Value = serde_json::from_str(txt).map_err(|e| {
        AppError::Upstream(format!("豆包生图接口返回了无效 JSON: {e}; body: {txt}"))
    })?;

    if let Some(msg) = ark_error_body_to_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let Some(data) = v.get("data").and_then(Value::as_array) else {
        return Err(AppError::Upstream(format!(
            "{}: 豆包生图响应缺少 data[]: {}",
            provider_label,
            txt.chars().take(400).collect::<String>()
        )));
    };

    let mut images: Vec<ImageResult> = Vec::new();
    for item in data {
        if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
            if let Ok(bytes) = B64.decode(b64.as_bytes()) {
                let mime = item
                    .get("mime_type")
                    .or_else(|| item.get("mimeType"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png")
                    .to_string();
                images.push(ImageResult { bytes, mime });
                continue;
            }
        }
        if let Some(u) = item.get("url").and_then(Value::as_str) {
            if let Some(img) = parse_data_url(u) {
                images.push(img);
            } else if u.starts_with("http://") || u.starts_with("https://") {
                images.push(fetch_remote_image(client, u, provider_label).await?);
            }
        }
    }

    if images.is_empty() {
        return Err(AppError::Upstream(format!(
            "{}: 豆包生图响应的 data[] 中没有可用的 url 或 b64_json",
            provider_label
        )));
    }

    Ok(GenerateResponse {
        images,
        text: None,
        usage: TokenUsage::default(),
    })
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
