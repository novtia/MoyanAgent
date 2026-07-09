//! Fetch the list of model ids a provider advertises via its `/models`
//! endpoint. Used by the settings "管理" model dialog so users can browse and
//! pull the upstream catalog into their local provider config.

use serde_json::Value;

use crate::ai::providers::{normalize_sdk, CLAUDE_SDK, GEMINI_SDK};
use crate::error::{AppError, AppResult};

const LIST_TIMEOUT_SECS: u64 = 30;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetch the upstream model ids for a provider config. Returns a de-duplicated,
/// order-preserving list of model id strings.
pub async fn fetch_models(sdk: &str, endpoint: &str, api_key: &str) -> AppResult<Vec<String>> {
    let sdk = normalize_sdk(sdk);
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(AppError::Invalid("API 地址不能为空。".into()));
    }
    let url = models_url(&sdk, endpoint);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(LIST_TIMEOUT_SECS))
        .build()?;

    let mut req = client.get(&url).header("Content-Type", "application/json");
    if sdk == GEMINI_SDK {
        req = req.header("x-goog-api-key", api_key);
    } else if sdk == CLAUDE_SDK {
        req = req
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION);
    } else {
        req = req.bearer_auth(api_key);
    }

    let resp = req.send().await?;
    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "拉取模型列表失败 HTTP {}: {}",
            status,
            upstream_error_message(&txt)
        )));
    }

    let v: Value = serde_json::from_str(&txt)
        .map_err(|err| AppError::Upstream(format!("无法解析模型列表响应: {err}")))?;
    let ids = parse_model_ids(&v);
    if ids.is_empty() {
        return Err(AppError::Upstream(
            "该供应商未返回任何模型，请检查 API 地址与密钥。".into(),
        ));
    }
    Ok(ids)
}

/// Derive the `/models` listing URL from a provider's configured request
/// endpoint (which usually points at chat/messages/images paths).
fn models_url(sdk: &str, endpoint: &str) -> String {
    let e = endpoint.trim().trim_end_matches('/');
    if sdk == GEMINI_SDK {
        if let Some(idx) = e.find("/models") {
            return format!("{}/models", &e[..idx]);
        }
        return format!("{}/models", e);
    }
    // OpenAI-compatible, Claude, Grok, Ark: strip the known request suffix and
    // append `/models` to the API base.
    const SUFFIXES: &[&str] = &[
        "/chat/completions",
        "/responses",
        "/messages",
        "/images/generations",
        "/images/edits",
        "/contents/generations/tasks",
        "/completions",
    ];
    for suffix in SUFFIXES {
        if let Some(idx) = e.rfind(suffix) {
            return format!("{}/models", &e[..idx]);
        }
    }
    if e.ends_with("/models") {
        return e.to_string();
    }
    format!("{}/models", e)
}

/// Extract model ids from the assorted shapes upstream providers return:
/// OpenAI/Claude `{ "data": [{ "id" }] }`, Gemini `{ "models": [{ "name" }] }`.
fn parse_model_ids(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for key in ["data", "models"] {
        if let Some(arr) = v.get(key).and_then(Value::as_array) {
            collect_ids(arr, &mut out, &mut seen);
        }
    }

    // Some gateways return a bare top-level array.
    if out.is_empty() {
        if let Some(arr) = v.as_array() {
            collect_ids(arr, &mut out, &mut seen);
        }
    }

    out
}

fn collect_ids(arr: &[Value], out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
    for item in arr {
        let raw = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .or_else(|| item.as_str());
        if let Some(raw) = raw {
            let id = raw.strip_prefix("models/").unwrap_or(raw).trim();
            if !id.is_empty() && seen.insert(id.to_string()) {
                out.push(id.to_string());
            }
        }
    }
}

/// Best-effort extraction of an error message from an upstream JSON error body.
fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .or_else(|| v.pointer("/error/type"))
            .or_else(|| v.pointer("/message"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}
