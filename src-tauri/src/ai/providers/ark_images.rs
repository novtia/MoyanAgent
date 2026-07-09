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
    parts.join("\n\n")
}

/// Seedream `size`（见 `豆包.md` / BytePlus Image generation API）：
/// - 方式 1：`宽x高` 精确像素（锁定比例）
/// - 方式 2：`1K`/`2K`/`3K`/`4K` + prompt 自然语言描述比例
/// 有 UI 比例时走方式 1，使用文档推荐像素表。
fn ark_size(model: &str, parameters: &GenerationParameters) -> String {
    let raw = parameters.image_size.trim();
    let ar = parameters.aspect_ratio.trim();
    let family = detect_seedream_family(model);
    if !ar.is_empty() && !ar.eq_ignore_ascii_case("auto") {
        let tier = normalize_size_tier(raw, family);
        if let Some((w, h)) = recommended_pixels(tier, ar) {
            return format_wxh(clamp_to_family(w, h, family));
        }
        if let Some((w, h)) = pixels_from_ratio(tier, ar, family) {
            return format_wxh(clamp_to_family(w, h, family));
        }
        return tier.to_string();
    }
    if looks_like_wxh(raw) {
        return raw.to_string();
    }
    normalize_size_tier(raw, family).to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeedreamFamily {
    /// seedream-5-0-pro：不支持 sequential/stream；size 总像素上限 2048²；宽高需为 16 倍数。
    Pro,
    /// seedream-5-0-lite：2K/3K/4K；总像素上限 4096²，下限约 2560×1440。
    Lite,
    /// seedream-4-5：2K/4K；总像素同 lite。
    V45,
    /// seedream-4-0：1K/2K/4K；总像素上限 4096²，下限 1280×720。
    V40,
    /// seedream-3-0-t2i：仅精确像素，默认约 1K。
    V30,
    /// Endpoint ID / 未知模型：按 4.0 兼容范围处理。
    Unknown,
}

fn detect_seedream_family(model: &str) -> SeedreamFamily {
    let m = model.to_ascii_lowercase().replace('_', "-");
    if m.contains("5-0-pro") || m.contains("5.0-pro") || m.contains("seedream-5-pro") {
        return SeedreamFamily::Pro;
    }
    if m.contains("5-0-lite") || m.contains("5.0-lite") || m.contains("seedream-5-lite") {
        return SeedreamFamily::Lite;
    }
    if m.contains("4-5") || m.contains("4.5") {
        return SeedreamFamily::V45;
    }
    if m.contains("4-0") || m.contains("4.0") || m.contains("seedream-4") {
        return SeedreamFamily::V40;
    }
    if m.contains("3-0") || m.contains("3.0") || m.contains("seedream-3") {
        return SeedreamFamily::V30;
    }
    SeedreamFamily::Unknown
}

fn looks_like_wxh(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    let mut parts = upper.split('X');
    let Some(w) = parts.next() else {
        return false;
    };
    let Some(h) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !w.is_empty()
        && !h.is_empty()
        && w.chars().all(|c| c.is_ascii_digit())
        && h.chars().all(|c| c.is_ascii_digit())
}

fn normalize_size_tier(image_size: &str, family: SeedreamFamily) -> &'static str {
    let s = image_size.trim();
    let requested = if s.is_empty() || s.eq_ignore_ascii_case("auto") {
        "2K"
    } else {
        match s.to_ascii_uppercase().as_str() {
            "1K" => "1K",
            "2K" => "2K",
            "3K" => "3K",
            "4K" => "4K",
            _ => "2K",
        }
    };
    // 各模型支持的档位不同；不支持时降到最接近的可用档。
    match family {
        SeedreamFamily::Pro => match requested {
            "1K" => "1K",
            _ => "2K", // pro 方式 2 仅 1K/2K；4K 像素会超其方式 1 上限
        },
        SeedreamFamily::Lite => match requested {
            "1K" => "2K",
            other => other,
        },
        SeedreamFamily::V45 => match requested {
            "1K" | "3K" => "2K",
            other => other,
        },
        SeedreamFamily::V40 | SeedreamFamily::Unknown => match requested {
            "3K" => "2K",
            other => other,
        },
        SeedreamFamily::V30 => "1K",
    }
}

fn parse_ratio(ratio: &str) -> Option<(u32, u32)> {
    let mut parts = ratio.split(':');
    let w = parts.next()?.trim().parse::<u32>().ok()?;
    let h = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() || w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

fn format_wxh((w, h): (u32, u32)) -> String {
    format!("{w}x{h}")
}

/// 文档推荐宽高表（`豆包.md` Seedream-5-0-pro / lite / 4-5 / 4-0）。
fn recommended_pixels(tier: &str, ratio: &str) -> Option<(u32, u32)> {
    let r = ratio.trim();
    let table: &[(&str, u32, u32)] = match tier {
        "1K" => &[
            ("1:1", 1024, 1024),
            ("4:3", 1152, 864),
            ("3:4", 864, 1152),
            ("16:9", 1312, 736),
            ("9:16", 736, 1312),
            ("3:2", 1248, 832),
            ("2:3", 832, 1248),
            ("21:9", 1568, 672),
        ],
        "3K" => &[
            ("1:1", 3072, 3072),
            ("4:3", 3456, 2592),
            ("3:4", 2592, 3456),
            ("16:9", 4096, 2304),
            ("9:16", 2304, 4096),
            ("3:2", 3744, 2496),
            ("2:3", 2496, 3744),
            ("21:9", 4704, 2016),
        ],
        "4K" => &[
            ("1:1", 4096, 4096),
            ("4:3", 4704, 3520),
            ("3:4", 3520, 4704),
            ("16:9", 5504, 3040),
            ("9:16", 3040, 5504),
            ("3:2", 4992, 3328),
            ("2:3", 3328, 4992),
            ("21:9", 6240, 2656),
        ],
        // 2K（各版本一致）
        _ => &[
            ("1:1", 2048, 2048),
            ("4:3", 2304, 1728),
            ("3:4", 1728, 2304),
            ("16:9", 2848, 1600),
            ("9:16", 1600, 2848),
            ("3:2", 2496, 1664),
            ("2:3", 1664, 2496),
            ("21:9", 3136, 1344),
        ],
    };
    table
        .iter()
        .find(|(k, _, _)| k.eq_ignore_ascii_case(r))
        .map(|(_, w, h)| (*w, *h))
}

fn family_max_pixels(family: SeedreamFamily) -> u64 {
    match family {
        SeedreamFamily::Pro | SeedreamFamily::V30 => 2048u64 * 2048,
        SeedreamFamily::Lite
        | SeedreamFamily::V45
        | SeedreamFamily::V40
        | SeedreamFamily::Unknown => 4096u64 * 4096,
    }
}

fn clamp_to_family(w: u32, h: u32, family: SeedreamFamily) -> (u32, u32) {
    let max_pixels = family_max_pixels(family);
    let mut nw = w.max(1);
    let mut nh = h.max(1);
    let pixels = (nw as u64) * (nh as u64);
    if pixels > max_pixels {
        let scale = (max_pixels as f64 / pixels as f64).sqrt();
        nw = ((nw as f64) * scale).floor() as u32;
        nh = ((nh as f64) * scale).floor() as u32;
    }
    // pro 要求宽高为 16 的倍数；其余取偶数更稳妥。
    let align = match family {
        SeedreamFamily::Pro => 16u32,
        _ => 2u32,
    };
    nw = (nw / align * align).max(align);
    nh = (nh / align * align).max(align);
    // 再保证不超总像素
    while (nw as u64) * (nh as u64) > max_pixels && (nw > align || nh > align) {
        if nw >= nh && nw > align {
            nw -= align;
        } else if nh > align {
            nh -= align;
        } else {
            break;
        }
    }
    (nw, nh)
}

fn pixels_from_ratio(tier: &str, ratio: &str, family: SeedreamFamily) -> Option<(u32, u32)> {
    let (rw, rh) = parse_ratio(ratio)?;
    let area = match tier {
        "1K" => 1024u64 * 1024,
        "3K" => 3072u64 * 3072,
        "4K" => 4096u64 * 4096,
        _ => 2048u64 * 2048,
    }
    .min(family_max_pixels(family)) as f64;
    let r = rw as f64 / rh as f64;
    let w = (area * r).sqrt().round() as u32;
    let h = (area / r).sqrt().round() as u32;
    Some((w, h))
}

fn build_body(request: &ChatRequest, prompt: &str) -> Value {
    // 不传 sequential_image_generation / stream：
    // seedream-5-0-pro 不支持这两个参数，传入会 400；默认即为单图、非流式。
    let mut body = json!({
        "model": request.model,
        "prompt": prompt,
        "size": ark_size(&request.model, &request.parameters),
        "response_format": "url",
        "watermark": false,
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
        videos: Vec::new(),
        text: None,
        thinking_content: None,
        usage: TokenUsage::default(),
        tool_calls: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::parameters::GenerationParameters;
    use crate::data::settings::ModelParamSettings;
    use serde_json::Map;

    fn params(aspect_ratio: &str, image_size: &str) -> GenerationParameters {
        GenerationParameters {
            aspect_ratio: aspect_ratio.into(),
            image_size: image_size.into(),
            model: ModelParamSettings::default(),
            video_mode: None,
            video_duration: None,
            video_resolution: None,
            generate_audio: None,
            watermark: None,
            camera_fixed: None,
            seed: None,
            custom: Map::new(),
        }
    }

    #[test]
    fn ark_size_auto_ratio_uses_tier() {
        assert_eq!(
            ark_size("doubao-seedream-4-0", &params("auto", "auto")),
            "2K"
        );
        assert_eq!(ark_size("doubao-seedream-4-0", &params("auto", "4K")), "4K");
        assert_eq!(ark_size("doubao-seedream-4-0", &params("", "1K")), "1K");
        // lite 不支持 1K → 降到 2K
        assert_eq!(ark_size("seedream-5-0-lite", &params("auto", "1K")), "2K");
        // 4.5 不支持 3K → 降到 2K
        assert_eq!(ark_size("seedream-4-5", &params("auto", "3K")), "2K");
    }

    #[test]
    fn ark_size_maps_ratio_to_doc_pixels() {
        assert_eq!(
            ark_size("seedream-5-0-lite", &params("16:9", "2K")),
            "2848x1600"
        );
        assert_eq!(
            ark_size("seedream-5-0-lite", &params("9:16", "2K")),
            "1600x2848"
        );
        assert_eq!(
            ark_size("seedream-5-0-lite", &params("1:1", "2K")),
            "2048x2048"
        );
        assert_eq!(
            ark_size("seedream-5-0-lite", &params("16:9", "3K")),
            "4096x2304"
        );
        // 4K 用文档表，不是 2K×2
        assert_eq!(
            ark_size("seedream-5-0-lite", &params("16:9", "4K")),
            "5504x3040"
        );
        // pro 1K 16:9 用文档表
        assert_eq!(
            ark_size("seedream-5-0-pro", &params("16:9", "1K")),
            "1312x736"
        );
    }

    #[test]
    fn ark_size_passthrough_wxh_when_auto_ratio() {
        assert_eq!(
            ark_size("seedream-4-0", &params("auto", "2560x1440")),
            "2560x1440"
        );
    }

    #[test]
    fn detect_seedream_family_from_model_id() {
        assert_eq!(
            detect_seedream_family("doubao-seedream-5-0-pro-250101"),
            SeedreamFamily::Pro
        );
        assert_eq!(
            detect_seedream_family("seedream-5-0-lite"),
            SeedreamFamily::Lite
        );
        assert_eq!(detect_seedream_family("seedream-4-5"), SeedreamFamily::V45);
        assert_eq!(detect_seedream_family("seedream-4-0"), SeedreamFamily::V40);
    }
}
