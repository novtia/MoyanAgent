//! Google Cloud Vertex AI Batch Prediction (`batchPredictionJobs`) adapter.
//!
//! Routes single-request `ChatRequest`s through GCS JSONL upload + Vertex
//! `batchPredictionJobs` + polling + `predictions.jsonl` download & cleanup.
//! Request JSON construction and response parsing are reused from `gemini.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::gemini::{
    build_body, google_auth_kind, parse_response, provider_label, upstream_error_message,
    GoogleAuthKind, GoogleAuthStyle,
};
use crate::ai::chat::{ChatRequest, GenerateResponse};
use crate::error::{AppError, AppResult};

const POLL_BACKOFF_SECS: &[u64] = &[15, 20, 30, 45, 60];
static RUN_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GcsLocation {
    pub bucket: String,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VertexBatchEndpoint {
    pub base_origin: String,
    pub api_version: String,
    pub project: String,
    pub location: String,
}

impl VertexBatchEndpoint {
    pub fn jobs_url(&self) -> String {
        format!(
            "{}/{}/projects/{}/locations/{}/batchPredictionJobs",
            self.base_origin, self.api_version, self.project, self.location
        )
    }

    pub fn job_url(&self, job_name: &str) -> String {
        let trimmed = job_name.trim().trim_start_matches('/');
        format!("{}/{}/{}", self.base_origin, self.api_version, trimmed)
    }

    pub fn job_cancel_url(&self, job_name: &str) -> String {
        format!("{}:cancel", self.job_url(job_name))
    }

    pub fn model_resource(&self, model: &str) -> String {
        let m = model.trim();
        if m.starts_with("publishers/") || m.starts_with("projects/") {
            m.to_string()
        } else {
            format!("publishers/google/models/{m}")
        }
    }
}

/// Current status snapshot for UI / trace display while a batch job is running.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchJobStatusSnapshot {
    pub job_name: String,
    pub state: String,
    pub model: String,
    pub elapsed_secs: u64,
}

pub(crate) type StatusEmitter = std::sync::Arc<dyn Fn(&BatchJobStatusSnapshot) + Send + Sync>;

tokio::task_local! {
    static BATCH_STATUS_EMITTER: StatusEmitter;
}

pub(crate) async fn with_status_emitter<F, T>(emitter: StatusEmitter, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    BATCH_STATUS_EMITTER.scope(emitter, fut).await
}

fn notify_status(snapshot: &BatchJobStatusSnapshot) {
    let _ = BATCH_STATUS_EMITTER.try_with(|emitter| emitter(snapshot));
}

pub(crate) fn parse_gcs_bucket(raw: &str) -> AppResult<GcsLocation> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "开启 Vertex 批量推理模式时，必须填写 GCS 存储桶路径（例如 gs://my-bucket/moyan-batch）。"
                .into(),
        ));
    }
    let without_scheme = trimmed
        .strip_prefix("gs://")
        .or_else(|| trimmed.strip_prefix("GS://"))
        .unwrap_or(trimmed);
    let (bucket, prefix) = match without_scheme.split_once('/') {
        Some((b, p)) => (b.trim(), p.trim().trim_matches('/')),
        None => (without_scheme.trim(), ""),
    };
    if bucket.is_empty() || bucket.chars().any(char::is_whitespace) {
        return Err(AppError::Config(format!(
            "GCS 存储桶路径无效：{trimmed}（示例：gs://my-bucket/moyan-batch）。"
        )));
    }
    Ok(GcsLocation {
        bucket: bucket.to_string(),
        prefix: prefix.to_string(),
    })
}

pub(crate) fn ensure_batch_bearer(api_key: &str) -> AppResult<&str> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "Vertex 批量推理需要服务账号 JSON 或 OAuth 访问令牌（ya29...）。".into(),
        ));
    }
    if google_auth_kind(trimmed, GoogleAuthStyle::VertexAuto) != GoogleAuthKind::Bearer {
        return Err(AppError::Config(
            "Vertex 批量推理与 GCS 上传不支持 API Key，请改用服务账号 JSON 或 OAuth 访问令牌（ya29...）。"
                .into(),
        ));
    }
    Ok(trimmed)
}

pub(crate) fn parse_vertex_endpoint(endpoint: &str) -> AppResult<VertexBatchEndpoint> {
    let trimmed = endpoint.trim();
    let (scheme_idx, after_scheme) = match trimmed.find("://") {
        Some(idx) => (idx, &trimmed[idx + 3..]),
        None => {
            return Err(AppError::Config(format!(
                "Vertex API 地址缺少协议头：{trimmed}"
            )));
        }
    };
    let (host, path) = match after_scheme.find('/') {
        Some(idx) => (&after_scheme[..idx], &after_scheme[idx..]),
        None => (after_scheme, ""),
    };
    if host.is_empty() {
        return Err(AppError::Config(format!(
            "Vertex API 地址缺少主机名：{trimmed}"
        )));
    }
    let base_origin = format!("{}://{}", &trimmed[..scheme_idx], host);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let api_version = segments
        .first()
        .copied()
        .filter(|s| s.starts_with('v'))
        .unwrap_or("v1")
        .to_string();

    let project = segments
        .windows(2)
        .find(|w| w[0] == "projects")
        .map(|w| w[1].trim())
        .filter(|s| !s.is_empty() && !s.contains('{'))
        .ok_or_else(|| {
            AppError::Config(
                "Vertex API 地址缺少有效的 GCP 项目 ID（projects/{project}）。".into(),
            )
        })?
        .to_string();

    let location = segments
        .windows(2)
        .find(|w| w[0] == "locations")
        .map(|w| w[1].trim())
        .filter(|s| !s.is_empty() && !s.contains('{'))
        .ok_or_else(|| {
            AppError::Config(
                "Vertex API 地址缺少有效的区域（locations/{location}）。".into(),
            )
        })?
        .to_string();

    Ok(VertexBatchEndpoint {
        base_origin,
        api_version,
        project,
        location,
    })
}

pub(crate) fn build_batch_input_jsonl(request: &ChatRequest) -> AppResult<String> {
    let mut body = build_body(request, false);
    // Vertex batch JSON import treats an OpenAPI `type: array` parameter as a
    // protobuf repeated field. A scalar (the shape Gemini often emits for a
    // one-item `path`) fails with "Repeated field must be imported as a JSON
    // array". Real-time generateContent accepts the scalar, so only the
    // batch payload is rewritten.
    coerce_batch_function_args(&mut body, request);
    let line = serde_json::to_string(&json!({ "request": body }))
        .map_err(|err| AppError::Upstream(format!("序列化 Vertex 批量请求失败: {err}")))?;
    Ok(format!("{line}\n"))
}

fn coerce_batch_function_args(body: &mut Value, request: &ChatRequest) {
    let Some(contents) = body.get_mut("contents").and_then(Value::as_array_mut) else {
        return;
    };
    for content in contents {
        let Some(parts) = content.get_mut("parts").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts {
            let Some(call) = part.get_mut("functionCall") else {
                continue;
            };
            let Some(name) = call.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let Some(schema) = request
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .map(|tool| tool.schema.clone())
            else {
                continue;
            };
            if let Some(args) = call.get_mut("args") {
                coerce_value_for_schema(args, &schema);
            }
        }
    }
}

fn schema_is_array(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) => kind == "array",
        Some(Value::Array(kinds)) => {
            kinds.iter().any(|kind| kind.as_str() == Some("array"))
                && kinds
                    .iter()
                    .all(|kind| matches!(kind.as_str(), Some("array" | "null")))
        }
        _ => false,
    }
}

fn coerce_value_for_schema(value: &mut Value, schema: &Value) {
    if schema_is_array(schema) && !value.is_array() && !value.is_null() {
        let mut inner = value.take();
        if let Some(items) = schema.get("items") {
            coerce_value_for_schema(&mut inner, items);
        }
        *value = json!([inner]);
        return;
    }
    if let Some(items) = schema.get("items") {
        if let Some(items_value) = value.as_array_mut() {
            let items = items.clone();
            for element in items_value {
                coerce_value_for_schema(element, &items);
            }
        }
        return;
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    let keys: Vec<String> = value
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    for key in keys {
        let Some(property) = properties.get(&key).cloned() else {
            continue;
        };
        if let Some(child) = value.get_mut(&key) {
            coerce_value_for_schema(child, &property);
        }
    }
}

pub(crate) fn next_poll_delay(attempt: usize) -> Duration {
    let idx = attempt.min(POLL_BACKOFF_SECS.len() - 1);
    Duration::from_secs(POLL_BACKOFF_SECS[idx])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JobPollDecision {
    Pending(String),
    Succeeded,
    Failed(String),
}

pub(crate) fn evaluate_job_state(job: &Value) -> JobPollDecision {
    let state = job
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("JOB_STATE_UNSPECIFIED")
        .trim();

    match state {
        "JOB_STATE_SUCCEEDED" => JobPollDecision::Succeeded,
        "JOB_STATE_FAILED"
        | "JOB_STATE_CANCELLED"
        | "JOB_STATE_EXPIRED"
        | "JOB_STATE_PARTIALLY_SUCCEEDED" => {
            let err_msg = job
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Vertex 批量任务结束但未成功：{state}"));
            JobPollDecision::Failed(err_msg)
        }
        _ => JobPollDecision::Pending(state.to_string()),
    }
}

pub(crate) fn parse_batch_prediction_jsonl(jsonl: &str) -> AppResult<GenerateResponse> {
    let mut first_error: Option<String> = None;
    for raw_line in jsonl.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let val: Value = serde_json::from_str(line).map_err(|err| {
            AppError::Upstream(format!("解析 Vertex 批量输出 JSONL 失败: {err}"))
        })?;
        if let Some(response) = val.get("response") {
            let resp_txt = serde_json::to_string(response).map_err(|err| {
                AppError::Upstream(format!("序列化批量 response 节点失败: {err}"))
            })?;
            return parse_response(&resp_txt);
        }
        if let Some(status_msg) = val
            .pointer("/status/message")
            .or_else(|| val.get("status").filter(|v| v.is_string()))
            .and_then(Value::as_str)
        {
            let trimmed = status_msg.trim();
            if !trimmed.is_empty() && first_error.is_none() {
                first_error = Some(trimmed.to_string());
            }
        }
    }

    if let Some(err) = first_error {
        return Err(AppError::Upstream(format!(
            "Vertex 批量推理单条请求失败: {err}"
        )));
    }
    Err(AppError::Upstream(
        "Vertex 批量推理输出 predictions.jsonl 中未包含有效 response。".into(),
    ))
}

fn url_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn next_run_id() -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{ts}-{seq}")
}

struct BatchJobGuard {
    client: reqwest::Client,
    token: String,
    bucket: String,
    cancel_url: Option<String>,
    cleanup_objects: Vec<String>,
    finished: bool,
}

impl BatchJobGuard {
    async fn cleanup_now(&mut self) {
        self.finished = true;
        let objects = std::mem::take(&mut self.cleanup_objects);
        for obj in objects {
            let _ = gcs_delete_object(&self.client, &self.token, &self.bucket, &obj).await;
        }
    }
}

impl Drop for BatchJobGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let client = self.client.clone();
        let token = self.token.clone();
        let bucket = self.bucket.clone();
        let cancel_url = self.cancel_url.take();
        let objects = std::mem::take(&mut self.cleanup_objects);
        if cancel_url.is_none() && objects.is_empty() {
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Some(url) = cancel_url {
                    let _ = client
                        .post(&url)
                        .bearer_auth(&token)
                        .header("Content-Type", "application/json")
                        .body("{}")
                        .send()
                        .await;
                }
                for obj in objects {
                    let _ = gcs_delete_object(&client, &token, &bucket, &obj).await;
                }
            });
        }
    }
}

async fn gcs_upload_jsonl(
    client: &reqwest::Client,
    token: &str,
    bucket: &str,
    object_name: &str,
    jsonl: String,
) -> AppResult<()> {
    let url = format!(
        "https://storage.googleapis.com/upload/storage/v1/b/{}/o?uploadType=media&name={}",
        url_encode_component(bucket),
        url_encode_component(object_name)
    );
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .header("Content-Type", "application/jsonl")
        .body(jsonl)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let txt = resp.text().await.unwrap_or_default();
        return Err(AppError::Upstream(format!(
            "GCS 上传批量输入失败 HTTP {status}: {}",
            upstream_error_message(&txt)
        )));
    }
    Ok(())
}

async fn gcs_list_objects(
    client: &reqwest::Client,
    token: &str,
    bucket: &str,
    prefix: &str,
) -> AppResult<Vec<String>> {
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{}/o?prefix={}",
        url_encode_component(bucket),
        url_encode_component(prefix)
    );
    let resp = client.get(&url).bearer_auth(token).send().await?;
    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "GCS 列出批量输出失败 HTTP {status}: {}",
            upstream_error_message(&txt)
        )));
    }
    let val: Value = serde_json::from_str(&txt).map_err(|err| {
        AppError::Upstream(format!("解析 GCS 对象列表失败: {err}"))
    })?;
    let names = val
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(names)
}

async fn gcs_download_object(
    client: &reqwest::Client,
    token: &str,
    bucket: &str,
    object_name: &str,
) -> AppResult<String> {
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
        url_encode_component(bucket),
        url_encode_component(object_name)
    );
    let resp = client.get(&url).bearer_auth(token).send().await?;
    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "GCS 下载批量输出失败 HTTP {status}: {}",
            upstream_error_message(&txt)
        )));
    }
    Ok(txt)
}

async fn gcs_delete_object(
    client: &reqwest::Client,
    token: &str,
    bucket: &str,
    object_name: &str,
) -> AppResult<()> {
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{}/o/{}",
        url_encode_component(bucket),
        url_encode_component(object_name)
    );
    let _ = client.delete(&url).bearer_auth(token).send().await?;
    Ok(())
}

pub(crate) async fn generate_vertex_batch(request: ChatRequest) -> AppResult<GenerateResponse> {
    let gcs = parse_gcs_bucket(&request.provider.batch_gcs_bucket)?;
    let token = ensure_batch_bearer(&request.provider.api_key)?.to_string();
    let endpoint = parse_vertex_endpoint(&request.provider.endpoint)?;
    let label = provider_label(&request);
    let input_jsonl = build_batch_input_jsonl(&request)?;

    let run_id = next_run_id();
    let base_prefix = if gcs.prefix.is_empty() {
        format!("runs/{run_id}")
    } else {
        format!("{}/runs/{run_id}", gcs.prefix)
    };
    let input_object = format!("{base_prefix}/input.jsonl");
    let output_prefix = format!("{base_prefix}/output");
    let input_uri = format!("gs://{}/{}", gcs.bucket, input_object);
    let output_uri = format!("gs://{}/{}/", gcs.bucket, output_prefix);

    let client = crate::ai::providers::build_chat_client()?;
    let mut guard = BatchJobGuard {
        client: client.clone(),
        token: token.clone(),
        bucket: gcs.bucket.clone(),
        cancel_url: None,
        cleanup_objects: vec![input_object.clone()],
        finished: false,
    };

    gcs_upload_jsonl(&client, &token, &gcs.bucket, &input_object, input_jsonl).await?;

    let create_body = json!({
        "displayName": format!("moyan-batch-{run_id}"),
        "model": endpoint.model_resource(&request.model),
        "inputConfig": {
            "instancesFormat": "jsonl",
            "gcsSource": {
                "uris": [input_uri],
            }
        },
        "outputConfig": {
            "predictionsFormat": "jsonl",
            "gcsDestination": {
                "outputUriPrefix": output_uri,
            }
        }
    });

    let create_resp = client
        .post(endpoint.jobs_url())
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .json(&create_body)
        .send()
        .await?;
    let create_status = create_resp.status();
    let create_txt = create_resp.text().await?;
    if !create_status.is_success() {
        guard.cleanup_now().await;
        return Err(AppError::Upstream(format!(
            "{} 批量任务创建失败 HTTP {}: {}",
            label,
            create_status,
            upstream_error_message(&create_txt)
        )));
    }
    let created: Value = serde_json::from_str(&create_txt).map_err(|err| {
        AppError::Upstream(format!("{label} 批量任务创建响应无法解析: {err}"))
    })?;
    let job_name = created
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Upstream(format!("{label} 批量任务创建响应缺少 name 字段。"))
        })?
        .to_string();

    let job_url = endpoint.job_url(&job_name);
    guard.cancel_url = Some(endpoint.job_cancel_url(&job_name));

    let started = Instant::now();
    let mut attempt = 0usize;
    loop {
        let poll_resp = client.get(&job_url).bearer_auth(&token).send().await?;
        let poll_status = poll_resp.status();
        let poll_txt = poll_resp.text().await?;
        if !poll_status.is_success() {
            guard.cleanup_now().await;
            return Err(AppError::Upstream(format!(
                "{} 批量任务轮询失败 HTTP {}: {}",
                label,
                poll_status,
                upstream_error_message(&poll_txt)
            )));
        }
        let job: Value = serde_json::from_str(&poll_txt).map_err(|err| {
            AppError::Upstream(format!("{label} 批量任务状态无法解析: {err}"))
        })?;
        match evaluate_job_state(&job) {
            JobPollDecision::Succeeded => {
                notify_status(&BatchJobStatusSnapshot {
                    job_name: job_name.clone(),
                    state: "JOB_STATE_SUCCEEDED".into(),
                    model: request.model.clone(),
                    elapsed_secs: started.elapsed().as_secs(),
                });
                break;
            }
            JobPollDecision::Failed(err_msg) => {
                guard.cleanup_now().await;
                return Err(AppError::Upstream(format!(
                    "{label} 批量任务失败: {err_msg}"
                )));
            }
            JobPollDecision::Pending(state) => {
                notify_status(&BatchJobStatusSnapshot {
                    job_name: job_name.clone(),
                    state,
                    model: request.model.clone(),
                    elapsed_secs: started.elapsed().as_secs(),
                });
                tokio::time::sleep(next_poll_delay(attempt)).await;
                attempt = attempt.saturating_add(1);
            }
        }
    }

    let output_objects = gcs_list_objects(&client, &token, &gcs.bucket, &output_prefix).await?;
    guard.cleanup_objects.extend(output_objects.iter().cloned());

    let prediction_obj = output_objects
        .iter()
        .find(|name| name.ends_with("predictions.jsonl"))
        .or_else(|| output_objects.iter().find(|name| name.ends_with(".jsonl")))
        .cloned()
        .ok_or_else(|| {
            AppError::Upstream(format!(
                "{label} 批量任务已完成，但在 {output_uri} 下未找到 predictions.jsonl。"
            ))
        });

    let result = match prediction_obj {
        Ok(obj) => {
            let jsonl = gcs_download_object(&client, &token, &gcs.bucket, &obj).await?;
            parse_batch_prediction_jsonl(&jsonl)
        }
        Err(err) => Err(err),
    };

    guard.cleanup_now().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat::{
        ChatRequest, PendingAssistantTurn, ProviderConfig, ProviderToolCall, ToolDefinition,
    };
    use crate::ai::parameters;

    fn sample_vertex_request() -> ChatRequest {
        ChatRequest {
            provider: ProviderConfig {
                id: "vertex".into(),
                name: "Vertex AI".into(),
                sdk: "vertex".into(),
                endpoint: "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/{model}:generateContent".into(),
                api_key: "ya29.test-token".into(),
                context_cache_enabled: false,
                safety_threshold: None,
                paygo: None,
                batch_mode: true,
                batch_gcs_bucket: "gs://my-bucket/moyan-batch/".into(),
            },
            model: "gemini-2.5-flash".into(),
            prompt: "hello".into(),
            attachments: Vec::new(),
            system_prompt: "sys".into(),
            history: Vec::new(),
            parameters: parameters::factory().build(
                "auto".into(),
                "auto".into(),
                crate::data::settings::ModelParamSettings::default(),
            ),
            tools: Vec::new(),
            tool_chain: Vec::new(),
            tool_results: Vec::new(),
            pending_assistant_turn: None,
            previous_response_id: None,
            context_cache_enabled: false,
            context_window: None,
            todo_snapshot: None,
            route_providers: Vec::new(),
            forced_tools: Vec::new(),
        }
    }

    #[test]
    fn parse_gcs_bucket_accepts_bucket_and_prefix() {
        let a = parse_gcs_bucket("gs://my-bucket/moyan/batch/").unwrap();
        assert_eq!(a.bucket, "my-bucket");
        assert_eq!(a.prefix, "moyan/batch");

        let b = parse_gcs_bucket("my-bucket").unwrap();
        assert_eq!(b.bucket, "my-bucket");
        assert_eq!(b.prefix, "");

        assert!(parse_gcs_bucket("   ").is_err());
        assert!(parse_gcs_bucket("gs:///bad").is_err());
    }

    #[test]
    fn ensure_batch_bearer_rejects_api_key() {
        assert!(ensure_batch_bearer("AIzaSyDummyKey").is_err());
        assert!(ensure_batch_bearer("").is_err());
        assert_eq!(ensure_batch_bearer("ya29.abc").unwrap(), "ya29.abc");
    }

    #[test]
    fn parse_vertex_endpoint_builds_batch_urls() {
        let regional = parse_vertex_endpoint(
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/{model}:generateContent",
        )
        .unwrap();
        assert_eq!(
            regional.jobs_url(),
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/batchPredictionJobs"
        );
        assert_eq!(
            regional.job_cancel_url("projects/my-proj/locations/us-central1/batchPredictionJobs/123"),
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/batchPredictionJobs/123:cancel"
        );
        assert_eq!(
            regional.model_resource("gemini-2.5-flash"),
            "publishers/google/models/gemini-2.5-flash"
        );

        let global = parse_vertex_endpoint(
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/{model}:generateContent",
        )
        .unwrap();
        assert_eq!(
            global.jobs_url(),
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/batchPredictionJobs"
        );
    }

    #[test]
    fn build_batch_input_jsonl_wraps_request_in_single_line() {
        let req = sample_vertex_request();
        let jsonl = build_batch_input_jsonl(&req).unwrap();
        assert!(jsonl.ends_with('\n'));
        assert_eq!(jsonl.lines().count(), 1);
        let parsed: Value = serde_json::from_str(jsonl.trim()).unwrap();
        assert!(parsed.get("request").is_some());
        assert_eq!(
            parsed.pointer("/request/contents/0/parts/0/text").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn batch_jsonl_wraps_scalar_array_parameters() {
        let mut req = sample_vertex_request();
        req.tools = vec![
            ToolDefinition {
                name: "Read".into(),
                description: "read files".into(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["path"]
                }),
            },
            ToolDefinition {
                name: "Edit".into(),
                description: "edit a file".into(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
        ];
        req.pending_assistant_turn = Some(PendingAssistantTurn {
            text: None,
            thinking_content: None,
            tool_calls: vec![
                ProviderToolCall::new("1", "Read", json!({ "path": "notes.md" })),
                ProviderToolCall::new("2", "Edit", json!({ "path": "notes.md" })),
                ProviderToolCall::new("3", "Read", json!({ "path": ["a.md", "b.md"] })),
            ],
        });

        let parsed: Value =
            serde_json::from_str(build_batch_input_jsonl(&req).unwrap().trim()).unwrap();
        let calls: Vec<&Value> = parsed
            .pointer("/request/contents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|content| content.get("parts").and_then(Value::as_array))
            .flatten()
            .filter_map(|part| part.get("functionCall"))
            .collect();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0]["args"]["path"], json!(["notes.md"]));
        assert_eq!(calls[1]["args"]["path"], json!("notes.md"));
        assert_eq!(calls[2]["args"]["path"], json!(["a.md", "b.md"]));
    }

    #[test]
    fn evaluate_job_state_handles_terminal_and_pending_states() {
        assert_eq!(
            evaluate_job_state(&json!({ "state": "JOB_STATE_QUEUED" })),
            JobPollDecision::Pending("JOB_STATE_QUEUED".into())
        );
        assert_eq!(
            evaluate_job_state(&json!({ "state": "JOB_STATE_RUNNING" })),
            JobPollDecision::Pending("JOB_STATE_RUNNING".into())
        );
        assert_eq!(
            evaluate_job_state(&json!({ "state": "JOB_STATE_SUCCEEDED" })),
            JobPollDecision::Succeeded
        );
        assert_eq!(
            evaluate_job_state(&json!({
                "state": "JOB_STATE_FAILED",
                "error": { "message": "quota exceeded" }
            })),
            JobPollDecision::Failed("quota exceeded".into())
        );
    }

    #[test]
    fn parse_batch_prediction_jsonl_extracts_text_and_tool_calls() {
        let jsonl = r#"{"status":"","processed_time":"2026-01-01T00:00:00Z","request":{},"response":{"candidates":[{"content":{"parts":[{"thought":true,"text":"先思考"},{"thoughtSignature":"sig-123","functionCall":{"name":"Read","args":{"path":"a.txt"}}}],"role":"model"}}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5,"totalTokenCount":15}}}"#;
        let parsed = parse_batch_prediction_jsonl(jsonl).unwrap();
        assert_eq!(parsed.thinking_content.as_deref(), Some("先思考"));
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "Read");
        assert_eq!(
            parsed.tool_calls[0].thought_signature.as_deref(),
            Some("sig-123")
        );
        assert_eq!(parsed.usage.total_tokens, Some(15));
    }

    #[test]
    fn parse_batch_prediction_jsonl_surfaces_row_error() {
        let jsonl = r#"{"status":{"code":3,"message":"invalid argument"}}"#;
        let err = parse_batch_prediction_jsonl(jsonl).unwrap_err();
        assert!(err.to_string().contains("invalid argument"));
    }
}
