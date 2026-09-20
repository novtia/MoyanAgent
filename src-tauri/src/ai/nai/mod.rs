//! NovelAI Diffusion V5 text-to-image client.
//!
//! Mirrors the reference `nai.py` payload: V5 models, `params_version` 4,
//! `v4_prompt` / `v4_negative_prompt`, optional character pins. The official
//! generate endpoint returns a ZIP of PNG bytes.

use std::io::{Cursor, Read};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

const IMAGE_API: &str = "https://image.novelai.net";
const USER_AGENT: &str = "MoyanAgent/1.0";
const GENERATE_TIMEOUT_SECS: u64 = 180;
const SUBSCRIPTION_TIMEOUT_SECS: u64 = 20;
const CONNECT_TIMEOUT_SECS: u64 = 20;
const MAX_CHARS: usize = 22;

pub const MODEL_FULL: &str = "nai-diffusion-5-full";
pub const MODEL_CURATED: &str = "nai-diffusion-5-curated";

pub const SAMPLERS: &[&str] = &[
    "k_euler_ancestral",
    "k_euler",
    "k_dpmpp_2s_ancestral",
    "k_dpmpp_2m",
    "k_dpmpp_2m_sde",
    "k_dpmpp_sde",
];

pub const SCHEDULES: &[&str] = &["karras", "native", "exponential", "polyexponential"];

pub const QUALITY_OFF: &str = "off";
pub const QUALITY_OFFICIAL: &str = "official";
pub const QUALITY_GALLERY: &str = "gallery";

pub const MODE_ANIME: &str = "anime";
pub const MODE_FURRY: &str = "furry";

pub const UC_HEAVY: &str = "heavy";
pub const UC_COMIC: &str = "comic";
pub const UC_NONE: &str = "none";
pub const UC_CUSTOM: &str = "custom";

const QUALITY_OFFICIAL_TAGS: &str = "very aesthetic, masterpiece, no text";
const QUALITY_GALLERY_TAGS: &str = "ultra complexity, very aesthetic, best quality, amazing quality, absurdres";
const UC_HEAVY_TEXT: &str = "lowres, artistic error, film grain, scan artifacts, worst quality, bad quality, jpeg artifacts, very displeasing, chromatic aberration, dithering, halftone, screentone, multiple views, logo, too many watermarks, negative space, blank page";
const UC_COMIC_TEXT: &str = "worst quality, bad quality, blurry, watermark, bad anatomy, extra fingers, ugly, fused face, cropped, jpeg artifacts, mutation, extra legs, missing fingers, poorly drawn hands, extra arms";

const TIER_NAMES: [&str; 4] = ["无订阅", "Tablet", "Scroll", "Opus"];

/// Persisted NovelAI tool settings (one JSON blob under the `novelai` key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NovelAiSettings {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_sampler")]
    pub sampler: String,
    #[serde(default = "default_schedule")]
    pub noise_schedule: String,
    #[serde(default = "default_steps")]
    pub steps: u32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub cfg_rescale: f64,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_v5_mode")]
    pub v5_mode: String,
    #[serde(default = "default_uc_preset")]
    pub uc_preset: String,
    #[serde(default)]
    pub uc: String,
    /// Weighted artist chain prepended before the model's prompt, e.g.
    /// `0.82::artist:name ::, 1.07::artist:other::`.
    #[serde(default)]
    pub artists: String,
    #[serde(default = "default_true")]
    pub straight_alpha: bool,
}

fn default_model() -> String {
    MODEL_FULL.into()
}
fn default_width() -> u32 {
    832
}
fn default_height() -> u32 {
    1216
}
fn default_sampler() -> String {
    "k_euler_ancestral".into()
}
fn default_schedule() -> String {
    "karras".into()
}
fn default_steps() -> u32 {
    28
}
fn default_scale() -> f64 {
    5.0
}
fn default_quality() -> String {
    QUALITY_GALLERY.into()
}
fn default_v5_mode() -> String {
    MODE_ANIME.into()
}
fn default_uc_preset() -> String {
    UC_HEAVY.into()
}
fn default_true() -> bool {
    true
}

impl Default for NovelAiSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: default_model(),
            width: default_width(),
            height: default_height(),
            sampler: default_sampler(),
            noise_schedule: default_schedule(),
            steps: default_steps(),
            scale: default_scale(),
            cfg_rescale: 0.0,
            quality: default_quality(),
            v5_mode: default_v5_mode(),
            uc_preset: default_uc_preset(),
            uc: String::new(),
            artists: String::new(),
            straight_alpha: true,
        }
    }
}

/// Clamp / coerce a settings blob loaded from disk or the settings UI.
pub fn normalize_settings(mut s: NovelAiSettings) -> NovelAiSettings {
    s.api_key = s.api_key.trim().to_string();
    if s.model != MODEL_FULL && s.model != MODEL_CURATED {
        s.model = default_model();
    }
    s.width = snap64(s.width as i64);
    s.height = snap64(s.height as i64);
    if !SAMPLERS.contains(&s.sampler.as_str()) {
        s.sampler = default_sampler();
    }
    if !SCHEDULES.contains(&s.noise_schedule.as_str()) {
        s.noise_schedule = default_schedule();
    }
    s.steps = s.steps.clamp(1, 50);
    if !s.scale.is_finite() {
        s.scale = default_scale();
    }
    s.scale = s.scale.clamp(0.0, 30.0);
    if !s.cfg_rescale.is_finite() {
        s.cfg_rescale = 0.0;
    }
    s.cfg_rescale = s.cfg_rescale.clamp(0.0, 1.0);
    if !matches!(s.quality.as_str(), "off" | "official" | "gallery") {
        s.quality = default_quality();
    }
    if s.v5_mode != MODE_FURRY {
        s.v5_mode = MODE_ANIME.into();
    }
    if s.uc_preset != UC_HEAVY
        && s.uc_preset != UC_COMIC
        && s.uc_preset != UC_NONE
        && s.uc_preset != UC_CUSTOM
    {
        s.uc_preset = default_uc_preset();
    }
    s.artists = s.artists.trim().to_string();
    s
}

#[derive(Debug, Clone, Default)]
pub struct CharacterPrompt {
    pub prompt: String,
    pub uc: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateRequest {
    pub prompt: String,
    pub uc: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seed: Option<i64>,
    pub n_samples: Option<u32>,
    pub characters: Vec<CharacterPrompt>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerateMeta {
    pub prompt: String,
    pub uc: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub sampler: String,
    pub scale: f64,
    pub seed: u32,
    pub noise_schedule: String,
    pub cfg_rescale: f64,
    pub n_samples: u32,
    pub v5_mode: String,
    pub quality: String,
    pub straight_alpha: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelAiSubscription {
    pub tier: i64,
    pub tier_name: String,
    pub active: bool,
    pub opus: bool,
    pub anlas: i64,
    pub anlas_fixed: i64,
    pub anlas_purchased: i64,
    pub usage_percent: Option<f64>,
    pub usage_negative: bool,
    pub expires_at: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelAiStatus {
    pub configured: bool,
    pub hint: String,
    pub subscription: Option<NovelAiSubscription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_status: Option<u16>,
}

pub fn snap64(value: i64) -> u32 {
    let lo = 64i64;
    let hi = 2048i64;
    let v = value.clamp(lo, hi);
    let snapped = (v / 64) * 64;
    snapped.max(lo) as u32
}

pub fn token_hint(token: &str) -> String {
    let token = token.trim();
    if token.len() < 8 {
        return "已保存".into();
    }
    format!("••••{}", &token[token.len() - 4..])
}

pub fn quality_tags(key: &str) -> &'static str {
    match key {
        QUALITY_OFFICIAL => QUALITY_OFFICIAL_TAGS,
        QUALITY_GALLERY => QUALITY_GALLERY_TAGS,
        _ => "",
    }
}

pub fn uc_preset_text(preset: &str) -> &'static str {
    match preset {
        UC_HEAVY => UC_HEAVY_TEXT,
        UC_COMIC => UC_COMIC_TEXT,
        _ => "",
    }
}

pub fn resolve_uc(settings: &NovelAiSettings, override_uc: Option<&str>) -> String {
    if let Some(uc) = override_uc.map(str::trim).filter(|s| !s.is_empty()) {
        return uc.to_string();
    }
    match settings.uc_preset.as_str() {
        UC_CUSTOM => settings.uc.trim().to_string(),
        other => uc_preset_text(other).to_string(),
    }
}

pub fn resolve_seed(raw: Option<i64>) -> u32 {
    match raw {
        Some(n) if n >= 0 => (n as u64 % (1u64 << 32)) as u32,
        _ => random_u32(),
    }
}

fn random_u32() -> u32 {
    (u128::from(ulid::Ulid::new()) & 0xFFFF_FFFF) as u32
}

fn join_parts(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim().trim_matches(',').trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// When a weighted `N::…::` span's body ends with a digit, insert a space
/// before the closing `::` so NovelAI does not treat that digit as a weight.
pub fn pad_numeric_closers(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    let mut rest = text;
    loop {
        let Some(first) = rest.find("::") else {
            out.push_str(rest);
            break;
        };
        let head = &rest[..first];
        let after_open = &rest[first + 2..];
        let Some(second_rel) = after_open.find("::") else {
            out.push_str(rest);
            break;
        };
        let weight = head
            .rsplit(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .next()
            .unwrap_or("");
        let looks_like_weight = !weight.is_empty()
            && weight.chars().any(|c| c.is_ascii_digit())
            && head.ends_with(weight);
        if !looks_like_weight {
            out.push_str(&rest[..first + 2]);
            rest = after_open;
            continue;
        }
        out.push_str(head);
        out.push_str("::");
        let body = after_open[..second_rel].trim_end();
        out.push_str(body);
        if body.ends_with(|c: char| c.is_ascii_digit()) {
            out.push(' ');
        }
        out.push_str("::");
        rest = &after_open[second_rel + 2..];
    }
    out
}

pub fn assemble_prompt(settings: &NovelAiSettings, user_prompt: &str) -> AppResult<String> {
    let prompt = pad_numeric_closers(user_prompt.trim());
    if prompt.is_empty() {
        return Err(AppError::Invalid("NovelAI: Prompt 不能为空".into()));
    }
    let prefix = if settings.v5_mode == MODE_FURRY {
        "fur dataset"
    } else {
        ""
    };
    let mut quality = quality_tags(&settings.quality).to_string();
    if !quality.is_empty() && prompt.to_ascii_lowercase().contains(&quality.to_ascii_lowercase())
    {
        quality.clear();
    }
    let artists = pad_numeric_closers(settings.artists.trim());
    if artists.is_empty() {
        return Ok(join_parts(&[prefix, &quality, &prompt]));
    }
    // User-configured artist chain first; the model's prompt follows so
    // weighted `N::artist:name::` spans stay a distinct block.
    let head = join_parts(&[prefix, &quality, &artists]);
    Ok(format!("{head}\n\n{prompt}"))
}

pub fn parse_characters(raw: &Value) -> Vec<CharacterPrompt> {
    let Some(arr) = raw.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr.iter().take(MAX_CHARS) {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let prompt = obj
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            continue;
        }
        let uc = obj
            .get("uc")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let x = obj
            .get("x")
            .and_then(Value::as_f64)
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let y = obj
            .get("y")
            .and_then(Value::as_f64)
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        out.push(CharacterPrompt { prompt, uc, x, y });
    }
    out
}

struct BuiltPayload {
    body: Value,
    meta: GenerateMeta,
}

pub fn build_payload(
    settings: &NovelAiSettings,
    request: &GenerateRequest,
) -> AppResult<(Value, GenerateMeta)> {
    let settings = normalize_settings(settings.clone());
    if settings.model != MODEL_FULL && settings.model != MODEL_CURATED {
        return Err(AppError::Invalid("NovelAI: 仅支持 NovelAI V5 模型".into()));
    }
    let prompt = assemble_prompt(&settings, &request.prompt)?;
    let uc = resolve_uc(&settings, request.uc.as_deref());
    let width = snap64(request.width.unwrap_or(settings.width) as i64);
    let height = snap64(request.height.unwrap_or(settings.height) as i64);
    let steps = settings.steps.clamp(1, 50);
    let scale = settings.scale.clamp(0.0, 30.0);
    let cfg_rescale = settings.cfg_rescale.clamp(0.0, 1.0);
    let n_samples = request.n_samples.unwrap_or(1).clamp(1, 4);
    let sampler = settings.sampler.clone();
    if !SAMPLERS.contains(&sampler.as_str()) {
        return Err(AppError::Invalid("NovelAI: 不支持的 sampler".into()));
    }
    let mut schedule = settings.noise_schedule.clone();
    if !SCHEDULES.contains(&schedule.as_str()) {
        return Err(AppError::Invalid("NovelAI: 不支持的 noise schedule".into()));
    }
    if schedule == "native" {
        schedule = "karras".into();
    }
    let seed = resolve_seed(request.seed);

    let characters: Vec<CharacterPrompt> = request
        .characters
        .iter()
        .cloned()
        .take(MAX_CHARS)
        .filter(|c| !c.prompt.trim().is_empty())
        .map(|mut c| {
            c.x = c.x.clamp(0.0, 1.0);
            c.y = c.y.clamp(0.0, 1.0);
            c
        })
        .collect();
    let use_coords = characters.iter().any(|c| (c.x - 0.5).abs() > f64::EPSILON || (c.y - 0.5).abs() > f64::EPSILON);

    let char_captions: Vec<Value> = characters
        .iter()
        .map(|c| {
            json!({
                "char_caption": c.prompt,
                "centers": [{ "x": c.x, "y": c.y }]
            })
        })
        .collect();
    let neg_captions: Vec<Value> = characters
        .iter()
        .map(|c| {
            json!({
                "char_caption": c.uc,
                "centers": [{ "x": c.x, "y": c.y }]
            })
        })
        .collect();

    let mut parameters = json!({
        "params_version": 4,
        "width": width,
        "height": height,
        "scale": scale,
        "sampler": sampler,
        "steps": steps,
        "seed": seed,
        "n_samples": n_samples,
        "extra_noise_seed": seed,
        "noise_schedule": schedule,
        "negative_prompt": uc,
        "cfg_rescale": cfg_rescale,
        "legacy": false,
        "legacy_uc": false,
        "legacy_v3_extend": false,
        "prefer_brownian": true,
        "deliberate_euler_ancestral_bug": false,
        "dynamic_thresholding": false,
        "controlnet_strength": 1,
        "use_coords": use_coords,
        "ucPresetId": "none",
        "qualityPresetId": "standard",
        "tag_hint_qt": 1,
        "tag_hint_uc_preset": 2,
        "normalize_reference_strength_multiple": true,
        "straight_alpha": settings.straight_alpha,
        "image_format": "png",
        "inpaintImg2ImgStrength": 1,
        "add_original_image": true,
        "v4_prompt": {
            "caption": {
                "base_caption": prompt,
                "char_captions": char_captions
            },
            "use_coords": use_coords,
            "use_order": true,
            "legacy_uc": false
        },
        "v4_negative_prompt": {
            "caption": {
                "base_caption": uc,
                "char_captions": neg_captions
            },
            "legacy_uc": false
        }
    });
    if !characters.is_empty() {
        let list: Vec<Value> = characters
            .iter()
            .map(|c| {
                json!({
                    "prompt": c.prompt,
                    "uc": c.uc,
                    "center": { "x": c.x, "y": c.y },
                    "enabled": true
                })
            })
            .collect();
        parameters["characterPrompts"] = Value::Array(list);
    }

    let body = json!({
        "input": prompt,
        "model": settings.model,
        "action": "generate",
        "parameters": parameters
    });
    let meta = GenerateMeta {
        prompt: prompt.clone(),
        uc,
        model: settings.model.clone(),
        width,
        height,
        steps,
        sampler,
        scale,
        seed,
        noise_schedule: schedule,
        cfg_rescale,
        n_samples,
        v5_mode: settings.v5_mode.clone(),
        quality: settings.quality.clone(),
        straight_alpha: settings.straight_alpha,
    };
    let built = BuiltPayload { body, meta };
    Ok((built.body, built.meta))
}

fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data.starts_with(b"\x89PNG\r\n\x1a\n")
}

pub fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 24 || !is_png(data) {
        return None;
    }
    let w = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(data[20..24].try_into().ok()?);
    Some((w, h))
}

pub fn unzip_images(blob: &[u8]) -> AppResult<Vec<Vec<u8>>> {
    if is_png(blob)
        || blob.starts_with(&[0xFF, 0xD8])
        || (blob.len() >= 12 && blob.starts_with(b"RIFF") && &blob[8..12] == b"WEBP")
    {
        return Ok(vec![blob.to_vec()]);
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(blob))
        .map_err(|e| AppError::Http(format!("NovelAI: 返回的压缩包无法打开：{e}")))?;
    let mut images = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| AppError::Http(format!("NovelAI: 读取压缩包失败：{e}")))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_ascii_lowercase();
        if !(name.ends_with(".png")
            || name.ends_with(".webp")
            || name.ends_with(".jpg")
            || name.ends_with(".jpeg"))
        {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        if !buf.is_empty() {
            images.push(buf);
        }
    }
    if images.is_empty() {
        return Err(AppError::Http("NovelAI 返回的压缩包里没有图片".into()));
    }
    Ok(images)
}

fn build_http_client(timeout_secs: u64) -> AppResult<reqwest::Client> {
    crate::ai::http_proxy::build_client(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS)),
    )
    .map_err(|e| AppError::Http(format!("NovelAI: 无法创建 HTTP 客户端：{e}")))
}

fn nai_http_error(status: u16, body: &[u8]) -> AppError {
    let fallback = String::from_utf8_lossy(body)
        .chars()
        .take(400)
        .collect::<String>();
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| {
                    if let Some(s) = m.as_str() {
                        Some(s.to_string())
                    } else {
                        m.get("message")
                            .and_then(Value::as_str)
                            .map(|s| s.to_string())
                    }
                })
                .or_else(|| {
                    v.get("error")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string())
                })
                .or_else(|| v.get("statusCode").map(|s| s.to_string()))
        })
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback);
    let message = if message.trim().is_empty() {
        "NovelAI 请求失败".to_string()
    } else {
        message
    };
    AppError::Http(format!("NovelAI HTTP {status}: {message}"))
}

async fn post_generate(token: &str, body: &Value) -> AppResult<Vec<u8>> {
    let client = build_http_client(GENERATE_TIMEOUT_SECS)?;
    let resp = client
        .post(format!("{IMAGE_API}/ai/generate-image"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "*/*")
        .header("User-Agent", USER_AGENT)
        .json(body)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("无法连接 NovelAI：{e}")))?;
    let status = resp.status();
    let bytes = resp.bytes().await?.to_vec();
    if !status.is_success() {
        return Err(nai_http_error(status.as_u16(), &bytes));
    }
    Ok(bytes)
}

pub async fn generate_images(
    settings: &NovelAiSettings,
    request: &GenerateRequest,
) -> AppResult<(Vec<Vec<u8>>, GenerateMeta)> {
    let token = settings.api_key.trim();
    if token.is_empty() {
        return Err(AppError::Config(
            "还没有配置 NovelAI Persistent API Token".into(),
        ));
    }
    let (payload, meta) = build_payload(settings, request)?;
    let blob = post_generate(token, &payload).await?;
    let images = unzip_images(&blob)?;
    Ok((images, meta))
}

pub async fn fetch_subscription(token: &str) -> AppResult<NovelAiSubscription> {
    let token = token.trim();
    if token.is_empty() {
        return Err(AppError::Config(
            "还没有配置 NovelAI Persistent API Token".into(),
        ));
    }
    let client = build_http_client(SUBSCRIPTION_TIMEOUT_SECS)?;
    let resp = client
        .get(format!("{IMAGE_API}/user/subscription"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "*/*")
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| AppError::Http(format!("无法连接 NovelAI：{e}")))?;
    let status = resp.status();
    let bytes = resp.bytes().await?.to_vec();
    if !status.is_success() {
        return Err(nai_http_error(status.as_u16(), &bytes));
    }
    parse_subscription(&bytes)
}

pub async fn status_for_token(token: &str) -> NovelAiStatus {
    let token = token.trim();
    if token.is_empty() {
        return NovelAiStatus {
            configured: false,
            hint: String::new(),
            subscription: None,
            error: None,
            error_status: None,
        };
    }
    match fetch_subscription(token).await {
        Ok(sub) => NovelAiStatus {
            configured: true,
            hint: token_hint(token),
            subscription: Some(sub),
            error: None,
            error_status: None,
        },
        Err(e) => {
            let (error, error_status) = match &e {
                AppError::Http(msg) => {
                    let code = msg
                        .strip_prefix("NovelAI HTTP ")
                        .and_then(|s| s.split(':').next())
                        .and_then(|s| s.trim().parse().ok());
                    (e.to_string(), code)
                }
                _ => (e.to_string(), None),
            };
            NovelAiStatus {
                configured: true,
                hint: token_hint(token),
                subscription: None,
                error: Some(error),
                error_status,
            }
        }
    }
}

fn parse_subscription(raw: &[u8]) -> AppResult<NovelAiSubscription> {
    let data: Value = serde_json::from_slice(raw)?;
    let training = data
        .get("trainingStepsLeft")
        .or_else(|| data.get("training_steps_left"))
        .cloned()
        .unwrap_or(Value::Null);
    let fixed = json_i64(
        &training,
        &["fixedTrainingStepsLeft", "fixed_training_steps_left"],
    );
    let purchased = json_i64(
        &training,
        &["purchasedTrainingSteps", "purchased_training_steps"],
    )
    .max(json_i64(
        &data,
        &["purchasedTrainingSteps", "purchased_training_steps"],
    ));
    let tier = json_i64(&data, &["tier"]);
    let fallback_name = TIER_NAMES
        .get(tier.clamp(0, 3) as usize)
        .copied()
        .unwrap_or("Tier");
    let tier_name = data
        .get("tierName")
        .or_else(|| data.get("tier_name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_name)
        .to_string();
    let usage = data.get("usage").cloned().unwrap_or(Value::Null);
    let usage_percent = usage
        .get("percent")
        .and_then(Value::as_f64)
        .or_else(|| usage.get("percent").and_then(Value::as_i64).map(|n| n as f64));
    Ok(NovelAiSubscription {
        tier,
        opus: tier >= 3 || tier_name.eq_ignore_ascii_case("opus"),
        tier_name,
        active: data
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        anlas: (fixed + purchased).max(0),
        anlas_fixed: fixed.max(0),
        anlas_purchased: purchased.max(0),
        usage_percent,
        usage_negative: usage
            .get("isNegative")
            .or_else(|| usage.get("is_negative"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        expires_at: data
            .get("expiresAt")
            .or_else(|| data.get("expires_at"))
            .cloned(),
    })
}

fn json_i64(v: &Value, keys: &[&str]) -> i64 {
    for key in keys {
        if let Some(n) = v.get(*key).and_then(Value::as_i64) {
            return n;
        }
        if let Some(n) = v.get(*key).and_then(Value::as_f64) {
            return n as i64;
        }
        if let Some(s) = v.get(*key).and_then(Value::as_str) {
            if let Ok(n) = s.parse::<i64>() {
                return n;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;
    use zip::ZipWriter;

    fn settings() -> NovelAiSettings {
        NovelAiSettings::default()
    }

    #[test]
    fn snap64_aligns_and_clamps() {
        assert_eq!(snap64(832), 832);
        assert_eq!(snap64(833), 832);
        assert_eq!(snap64(10), 64);
        assert_eq!(snap64(3000), 2048);
        assert_eq!(snap64(64), 64);
    }

    #[test]
    fn assemble_prompt_adds_gallery_quality() {
        let s = settings();
        let prompt = assemble_prompt(&s, "1girl, solo").unwrap();
        assert!(prompt.starts_with(QUALITY_GALLERY_TAGS));
        assert!(prompt.contains("1girl, solo"));
        assert!(!prompt.contains("fur dataset"));
    }

    #[test]
    fn assemble_prompt_skips_quality_when_already_present() {
        let s = settings();
        let prompt = assemble_prompt(&s, QUALITY_GALLERY_TAGS).unwrap();
        assert_eq!(prompt, QUALITY_GALLERY_TAGS);
    }

    #[test]
    fn assemble_prompt_furry_prefix() {
        let mut s = settings();
        s.v5_mode = MODE_FURRY.into();
        s.quality = QUALITY_OFF.into();
        let prompt = assemble_prompt(&s, "wolf, solo").unwrap();
        assert_eq!(prompt, "fur dataset, wolf, solo");
    }

    #[test]
    fn assemble_prompt_rejects_empty() {
        let err = assemble_prompt(&settings(), "   ").unwrap_err();
        assert!(err.to_string().contains("Prompt"));
    }

    #[test]
    fn assemble_prompt_prepends_artist_chain() {
        let mut s = settings();
        s.quality = QUALITY_OFF.into();
        s.artists = "0.82::artist:bm94199 ::, 1.07::artist:hiro_(dismaless)::, 0.81::artist:96yottea::, 1.30::artist:chamchami::".into();
        let prompt = assemble_prompt(&s, "1girl, solo").unwrap();
        let expected = "0.82::artist:bm94199 ::, 1.07::artist:hiro_(dismaless)::, 0.81::artist:96yottea::, 1.30::artist:chamchami::\n\n1girl, solo";
        assert_eq!(prompt, expected);
    }

    #[test]
    fn pad_numeric_closers_inserts_space() {
        let out = pad_numeric_closers("0.99::artist:foo2::, 1girl");
        assert!(out.contains("foo2 ::"), "{out}");
    }

    #[test]
    fn only_v5_models() {
        let mut s = settings();
        s.model = "nai-diffusion-4-full".into();
        let req = GenerateRequest {
            prompt: "1girl".into(),
            ..Default::default()
        };
        // normalize_settings remaps unknown models to V5 full before generate,
        // but build_payload still accepts only the two V5 ids after normalize.
        let (body, _) = build_payload(&s, &req).unwrap();
        assert_eq!(body["model"], MODEL_FULL);
    }

    #[test]
    fn payload_includes_v4_prompt_and_characters() {
        let s = settings();
        let req = GenerateRequest {
            prompt: "classroom".into(),
            uc: Some("lowres".into()),
            seed: Some(42),
            n_samples: Some(1),
            characters: vec![CharacterPrompt {
                prompt: "girl, solo".into(),
                uc: String::new(),
                x: 0.28,
                y: 0.48,
            }],
            ..Default::default()
        };
        let (body, meta) = build_payload(&s, &req).unwrap();
        assert_eq!(body["action"], "generate");
        assert_eq!(body["parameters"]["params_version"], 4);
        assert_eq!(body["parameters"]["seed"], 42);
        assert_eq!(meta.seed, 42);
        assert_eq!(body["parameters"]["use_coords"], true);
        let chars = body["parameters"]["characterPrompts"].as_array().unwrap();
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0]["prompt"], "girl, solo");
        assert_eq!(
            body["parameters"]["v4_prompt"]["caption"]["base_caption"],
            body["input"]
        );
        assert_eq!(body["parameters"]["negative_prompt"], "lowres");
    }

    #[test]
    fn unzip_images_reads_png_from_zip() {
        let png = {
            let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
            v.extend_from_slice(&[0u8; 16]);
            v
        };
        let cursor = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(cursor);
        zip.start_file(
            "image.png",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(&png).unwrap();
        let cursor = zip.finish().unwrap();
        let images = unzip_images(&cursor.into_inner()).unwrap();
        assert_eq!(images.len(), 1);
        assert!(is_png(&images[0]));
    }

    #[test]
    fn unzip_images_accepts_bare_png() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0u8; 16]);
        let images = unzip_images(&png).unwrap();
        assert_eq!(images.len(), 1);
    }

    #[test]
    fn png_size_reads_ihdr() {
        let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
        data.extend_from_slice(&[0, 0, 0, 13]); // length (ignored)
        data.extend_from_slice(b"IHDR");
        // overwrite: png_size reads bytes 16..24 as width/height
        data.resize(16, 0);
        data.extend_from_slice(&832u32.to_be_bytes());
        data.extend_from_slice(&1216u32.to_be_bytes());
        assert_eq!(png_size(&data), Some((832, 1216)));
    }

    #[test]
    fn resolve_uc_prefers_override() {
        let mut s = settings();
        s.uc_preset = UC_HEAVY.into();
        assert_eq!(resolve_uc(&s, Some("custom uc")), "custom uc");
        assert!(resolve_uc(&s, None).contains("lowres"));
        s.uc_preset = UC_CUSTOM.into();
        s.uc = "  my uc  ".into();
        assert_eq!(resolve_uc(&s, None), "my uc");
    }
}
