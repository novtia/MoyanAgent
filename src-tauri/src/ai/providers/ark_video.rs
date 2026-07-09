//! 火山方舟 / BytePlus Seedance 视频生成 API。
//!
//! 创建任务是异步接口：先 POST `…/contents/generations/tasks`，再轮询
//! `GET …/contents/generations/tasks/{id}`，成功后立即下载临时视频 URL。

use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use reqwest::{Client, StatusCode};
use serde_json::{json, Map, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, MediaResult};
use crate::ai::providers::{ChatProvider, ProviderFuture, ARK_VIDEO_SDK};
use crate::ai::tokens;
use crate::error::{AppError, AppResult};

const CREATE_TIMEOUT_SECS: u64 = 90;
const POLL_REQUEST_TIMEOUT_SECS: u64 = 45;
const DOWNLOAD_TIMEOUT_SECS: u64 = 15 * 60;
const TOTAL_POLL_TIMEOUT_SECS: u64 = 15 * 60;
const POLL_INTERVAL_SECS: u64 = 3;
const MAX_CREATE_ATTEMPTS: usize = 3;
const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_VIDEO_BYTES: usize = 512 * 1024 * 1024;

pub struct ArkVideoProvider;

impl ArkVideoProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for ArkVideoProvider {
    fn sdk(&self) -> &'static str {
        ARK_VIDEO_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let tasks_url = resolve_tasks_url(&request.provider.endpoint);
    if tasks_url.is_empty() {
        return Err(AppError::Config(
            "豆包生视频需要填写 Endpoint（例如 https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks）"
                .into(),
        ));
    }

    let body = build_body(&request)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()?;
    let task_id = create_task(
        &client,
        &tasks_url,
        &request.provider.api_key,
        &body,
        &provider_label(&request),
    )
    .await?;

    // If the provider future is dropped or polling/downloading fails, this
    // guard makes a best-effort DELETE in the background. A completed download
    // disarms it; Ark may reject DELETE once a task is already running.
    let mut cleanup = TaskCleanup::new(
        client.clone(),
        format!("{}/{}", tasks_url.trim_end_matches('/'), task_id),
        request.provider.api_key.clone(),
    );
    let task = poll_task(
        &client,
        &tasks_url,
        &task_id,
        &request.provider.api_key,
        &provider_label(&request),
    )
    .await?;

    let video_url = extract_video_url(&task).ok_or_else(|| {
        AppError::Upstream(format!(
            "{}: 任务已成功，但响应缺少 content.video_url",
            provider_label(&request)
        ))
    })?;
    let media = download_video(&client, video_url, &provider_label(&request), &task).await?;
    cleanup.disarm();

    Ok(GenerateResponse {
        images: Vec::new(),
        videos: vec![media],
        text: None,
        thinking_content: None,
        usage: tokens::extract_usage(&task),
        tool_calls: Vec::new(),
    })
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

/// Accept a full tasks endpoint or a common Ark request/base URL.
fn resolve_tasks_url(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return String::new();
    }
    const KNOWN_SUFFIXES: &[&str] = &[
        "/contents/generations/tasks",
        "/images/generations",
        "/chat/completions",
        "/responses",
    ];
    for suffix in KNOWN_SUFFIXES {
        if let Some(index) = endpoint.rfind(suffix) {
            return format!(
                "{}/contents/generations/tasks",
                endpoint[..index].trim_end_matches('/')
            );
        }
    }
    if endpoint.ends_with("/api/v3") {
        format!("{endpoint}/contents/generations/tasks")
    } else {
        format!("{endpoint}/contents/generations/tasks")
    }
}

fn build_body(request: &ChatRequest) -> AppResult<Value> {
    let mode = request.parameters.video_mode.as_deref().unwrap_or("text");
    let mut content = Vec::new();
    let prompt = request.prompt.trim();
    if !prompt.is_empty() {
        content.push(json!({ "type": "text", "text": prompt }));
    }

    let mut image_count = 0usize;
    let mut audio_count = 0usize;
    let mut video_count = 0usize;
    for (index, attachment) in request.attachments.iter().enumerate() {
        let mime = attachment.mime.to_ascii_lowercase();
        if mime.starts_with("image/") {
            image_count += 1;
            let role = attachment
                .media_role
                .as_deref()
                .or_else(|| inferred_image_role(mode, index));
            let mut item = json!({
                "type": "image_url",
                "image_url": { "url": attachment_url(attachment)? },
            });
            if let Some(role) = role {
                item.as_object_mut()
                    .expect("image content object")
                    .insert("role".into(), json!(role));
            }
            content.push(item);
        } else if mime.starts_with("audio/") {
            audio_count += 1;
            content.push(json!({
                "type": "audio_url",
                "audio_url": { "url": attachment_url(attachment)? },
                "role": attachment.media_role.as_deref().unwrap_or("reference_audio"),
            }));
        } else if mime.starts_with("video/") {
            video_count += 1;
            let Some(url) = attachment
                .source_url
                .as_deref()
                .map(str::trim)
                .filter(|url| is_remote_or_asset_url(url))
            else {
                return Err(AppError::Invalid(
                    "Seedance 参考视频仅支持公网 http(s) URL 或 asset:// 资源，不能直接上传本地视频"
                        .into(),
                ));
            };
            content.push(json!({
                "type": "video_url",
                "video_url": { "url": url },
                "role": attachment.media_role.as_deref().unwrap_or("reference_video"),
            }));
        } else {
            return Err(AppError::Invalid(format!(
                "Seedance 不支持附件类型：{}",
                attachment.mime
            )));
        }
    }

    validate_mode(
        mode,
        prompt,
        image_count,
        audio_count,
        video_count,
        &request.model,
    )?;

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("content".into(), Value::Array(content));

    let ratio = request.parameters.aspect_ratio.trim();
    body.insert(
        "ratio".into(),
        json!(if ratio.is_empty() || ratio == "auto" {
            "adaptive"
        } else {
            ratio
        }),
    );
    let resolution = request
        .parameters
        .video_resolution
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("720p");
    if resolution == "4k"
        && is_known_seedance_model(&request.model)
        && !is_seedance_2(&request.model)
    {
        return Err(AppError::Invalid(
            "4k 分辨率仅支持 Seedance 2.0 系列".into(),
        ));
    }
    body.insert("resolution".into(), json!(resolution));

    let duration = request.parameters.video_duration.unwrap_or(5);
    validate_duration(&request.model, duration)?;
    body.insert("duration".into(), json!(duration));
    body.insert(
        "watermark".into(),
        json!(request.parameters.watermark.unwrap_or(false)),
    );
    if supports_generated_audio(&request.model) {
        body.insert(
            "generate_audio".into(),
            json!(request.parameters.generate_audio.unwrap_or(true)),
        );
    }
    if !is_seedance_2(&request.model) {
        if let Some(value) = request.parameters.camera_fixed {
            body.insert("camera_fixed".into(), json!(value));
        }
        if let Some(value) = request.parameters.seed {
            body.insert("seed".into(), json!(value));
        }
    }
    body.insert("execution_expires_after".into(), json!(3600));

    let body = Value::Object(body);
    let encoded_len = serde_json::to_vec(&body)
        .map_err(|error| AppError::Invalid(format!("无法序列化视频生成请求：{error}")))?
        .len();
    if encoded_len > MAX_REQUEST_BYTES {
        return Err(AppError::Invalid(format!(
            "Seedance 请求体约 {:.1} MB，超过 64 MB 上限；请减少参考媒体或压缩文件",
            encoded_len as f64 / (1024.0 * 1024.0)
        )));
    }
    Ok(body)
}

fn inferred_image_role(mode: &str, index: usize) -> Option<&'static str> {
    match mode {
        "first_frame" => Some("first_frame"),
        "first_last" if index == 0 => Some("first_frame"),
        "first_last" => Some("last_frame"),
        "reference" => Some("reference_image"),
        _ => None,
    }
}

fn attachment_url(attachment: &AttachmentBytes) -> AppResult<String> {
    if let Some(url) = attachment
        .source_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return Ok(url.to_string());
    }
    if attachment.bytes.is_empty() {
        return Err(AppError::Invalid(format!(
            "{} 附件没有本地内容或可用 URL",
            attachment.mime
        )));
    }
    Ok(format!(
        "data:{};base64,{}",
        attachment.mime.to_ascii_lowercase(),
        B64.encode(&attachment.bytes)
    ))
}

fn validate_mode(
    mode: &str,
    prompt: &str,
    images: usize,
    audio: usize,
    videos: usize,
    model: &str,
) -> AppResult<()> {
    match mode {
        "text" => {
            if prompt.is_empty() {
                return Err(AppError::Invalid("文生视频需要非空提示词".into()));
            }
            if images + audio + videos > 0 {
                return Err(AppError::Invalid("文生视频模式不能包含媒体附件".into()));
            }
        }
        "first_frame" => {
            if images != 1 || audio + videos != 0 {
                return Err(AppError::Invalid(
                    "首帧图生视频模式需要且仅需要 1 张图片".into(),
                ));
            }
        }
        "first_last" => {
            if images != 2 || audio + videos != 0 {
                return Err(AppError::Invalid(
                    "首尾帧生视频模式需要且仅需要 2 张图片".into(),
                ));
            }
        }
        "reference" => {
            if is_known_seedance_model(model) && !is_seedance_2(model) {
                return Err(AppError::Invalid(
                    "多模态参考生视频仅支持 Seedance 2.0 系列".into(),
                ));
            }
            if images > 9 {
                return Err(AppError::Invalid("参考图片最多 9 张".into()));
            }
            if videos > 3 {
                return Err(AppError::Invalid("参考视频最多 3 个".into()));
            }
            if audio > 3 {
                return Err(AppError::Invalid("参考音频最多 3 段".into()));
            }
            if images + videos == 0 {
                return Err(AppError::Invalid(
                    "多模态参考不能仅输入音频，至少需要 1 张参考图或 1 个参考视频".into(),
                ));
            }
        }
        other => {
            return Err(AppError::Invalid(format!("未知视频生成模式：{other}")));
        }
    }
    Ok(())
}

fn validate_duration(model: &str, duration: i64) -> AppResult<()> {
    if duration == -1
        && (is_seedance_2(model) || is_seedance_15(model) || !is_known_seedance_model(model))
    {
        return Ok(());
    }
    let valid = if is_seedance_2(model) {
        (4..=15).contains(&duration)
    } else if is_seedance_15(model) {
        (4..=12).contains(&duration)
    } else if is_known_seedance_model(model) {
        (2..=12).contains(&duration)
    } else {
        (2..=15).contains(&duration)
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "当前 Seedance 模型不支持 {duration} 秒视频"
        )))
    }
}

fn normalized_model(model: &str) -> String {
    model.to_ascii_lowercase().replace('_', "-")
}

fn is_seedance_2(model: &str) -> bool {
    let model = normalized_model(model);
    model.contains("seedance-2") || model.contains("seedance2")
}

fn is_seedance_15(model: &str) -> bool {
    let model = normalized_model(model);
    model.contains("seedance-1-5") || model.contains("seedance1-5")
}

fn is_known_seedance_model(model: &str) -> bool {
    normalized_model(model).contains("seedance")
}

fn supports_generated_audio(model: &str) -> bool {
    is_seedance_2(model) || is_seedance_15(model) || !is_known_seedance_model(model)
}

fn is_remote_or_asset_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("asset://")
}

async fn create_task(
    client: &Client,
    url: &str,
    api_key: &str,
    body: &Value,
    provider_label: &str,
) -> AppResult<String> {
    for attempt in 1..=MAX_CREATE_ATTEMPTS {
        let response = client
            .post(url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(CREATE_TIMEOUT_SECS))
            .json(body)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                if attempt < MAX_CREATE_ATTEMPTS && should_retry_transport(&error) {
                    retry_sleep(attempt).await;
                    continue;
                }
                return Err(error.into());
            }
        };
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            let value: Value = serde_json::from_str(&text).map_err(|error| {
                AppError::Upstream(format!(
                    "{provider_label}: 创建视频任务返回无效 JSON：{error}"
                ))
            })?;
            if let Some(id) = value
                .get("id")
                .or_else(|| value.get("task_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            {
                return Ok(id.to_string());
            }
            return Err(AppError::Upstream(format!(
                "{provider_label}: 创建视频任务响应缺少 id：{}",
                text.chars().take(500).collect::<String>()
            )));
        }
        if attempt < MAX_CREATE_ATTEMPTS && retryable_status(status) {
            retry_sleep(attempt).await;
            continue;
        }
        return Err(AppError::Upstream(format!(
            "{provider_label} 创建视频任务 HTTP {status}: {}",
            upstream_error_message(&text)
        )));
    }
    unreachable!("create task attempts always return");
}

async fn poll_task(
    client: &Client,
    tasks_url: &str,
    task_id: &str,
    api_key: &str,
    provider_label: &str,
) -> AppResult<Value> {
    let url = format!("{}/{}", tasks_url.trim_end_matches('/'), task_id);
    let started = Instant::now();
    loop {
        if started.elapsed() >= Duration::from_secs(TOTAL_POLL_TIMEOUT_SECS) {
            return Err(AppError::Upstream(format!(
                "{provider_label}: 视频生成等待超过 {} 分钟",
                TOTAL_POLL_TIMEOUT_SECS / 60
            )));
        }
        let response = client
            .get(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(POLL_REQUEST_TIMEOUT_SECS))
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await?;
                if !status.is_success() {
                    if retryable_status(status) {
                        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                        continue;
                    }
                    return Err(AppError::Upstream(format!(
                        "{provider_label} 查询视频任务 HTTP {status}: {}",
                        upstream_error_message(&text)
                    )));
                }
                let value: Value = serde_json::from_str(&text).map_err(|error| {
                    AppError::Upstream(format!(
                        "{provider_label}: 查询视频任务返回无效 JSON：{error}"
                    ))
                })?;
                let task_status = value
                    .get("status")
                    .or_else(|| value.get("task_status"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match task_status.as_str() {
                    "succeeded" | "completed" => return Ok(value),
                    "queued" | "running" | "pending" | "processing" | "" => {}
                    "failed" | "expired" | "cancelled" | "canceled" => {
                        return Err(AppError::Upstream(format!(
                            "{provider_label}: 视频任务状态为 {task_status}: {}",
                            ark_error_body_to_message(&value)
                                .unwrap_or_else(|| "上游未提供详细错误".into())
                        )));
                    }
                    other => {
                        return Err(AppError::Upstream(format!(
                            "{provider_label}: 未知视频任务状态 {other}"
                        )));
                    }
                }
            }
            Err(error) if should_retry_transport(&error) => {}
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
    }
}

fn extract_video_url(task: &Value) -> Option<&str> {
    [
        "/content/video_url",
        "/data/content/video_url",
        "/result/download_url",
        "/result/video_url",
        "/result_url",
    ]
    .into_iter()
    .find_map(|pointer| task.pointer(pointer).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
}

async fn download_video(
    client: &Client,
    url: &str,
    provider_label: &str,
    task: &Value,
) -> AppResult<MediaResult> {
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "{provider_label}: 下载生成视频 HTTP {status}: {}",
            text.chars().take(300).collect::<String>()
        )));
    }
    if response
        .content_length()
        .map(|size| size > MAX_VIDEO_BYTES as u64)
        .unwrap_or(false)
    {
        return Err(AppError::Upstream(format!(
            "{provider_label}: 生成视频超过 512 MB 下载上限"
        )));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| value.starts_with("video/"))
        .unwrap_or("video/mp4")
        .to_string();
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_VIDEO_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_VIDEO_BYTES {
            return Err(AppError::Upstream(format!(
                "{provider_label}: 生成视频超过 512 MB 下载上限"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err(AppError::Upstream(format!(
            "{provider_label}: 生成视频下载结果为空"
        )));
    }
    Ok(MediaResult {
        bytes,
        mime,
        width: None,
        height: None,
        duration: task.get("duration").and_then(Value::as_f64),
    })
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn should_retry_transport(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

async fn retry_sleep(attempt: usize) {
    tokio::time::sleep(Duration::from_millis(500 * (1u64 << (attempt - 1)))).await;
}

fn upstream_error_message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| ark_error_body_to_message(&value))
        .unwrap_or_else(|| text.chars().take(500).collect())
}

fn ark_error_body_to_message(value: &Value) -> Option<String> {
    value
        .pointer("/error/message")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

struct TaskCleanup {
    client: Client,
    url: String,
    api_key: String,
    armed: bool,
}

impl TaskCleanup {
    fn new(client: Client, url: String, api_key: String) -> Self {
        Self {
            client,
            url,
            api_key,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TaskCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let client = self.client.clone();
        let url = self.url.clone();
        let api_key = self.api_key.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = client
                    .delete(url)
                    .bearer_auth(api_key)
                    .timeout(Duration::from_secs(15))
                    .send()
                    .await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_domestic_and_byteplus_endpoints() {
        assert_eq!(
            resolve_tasks_url("https://ark.cn-beijing.volces.com/api/v3"),
            "https://ark.cn-beijing.volces.com/api/v3/contents/generations/tasks"
        );
        assert_eq!(
            resolve_tasks_url("https://ark.ap-southeast.bytepluses.com/api/v3/images/generations"),
            "https://ark.ap-southeast.bytepluses.com/api/v3/contents/generations/tasks"
        );
    }

    #[test]
    fn validates_reference_limits() {
        assert!(validate_mode("reference", "", 1, 3, 3, "doubao-seedance-2-0-260128").is_ok());
        assert!(validate_mode("reference", "", 0, 1, 0, "doubao-seedance-2-0-260128").is_err());
    }

    #[test]
    fn endpoint_ids_defer_family_validation_to_ark() {
        assert!(validate_mode("reference", "", 1, 0, 0, "ep-20260710-example").is_ok());
        assert!(validate_duration("ep-20260710-example", -1).is_ok());
        assert!(validate_duration("ep-20260710-example", 15).is_ok());
        assert!(validate_mode(
            "reference",
            "",
            1,
            0,
            0,
            "doubao-seedance-1-5-pro-251215"
        )
        .is_err());
    }
}
