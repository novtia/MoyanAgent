//! Fetch the model catalog a provider advertises via its `/models` endpoint.
//! Used by the settings "管理" dialog so users can browse and import models
//! with context window, pricing, and capability metadata when available.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::providers::{normalize_sdk, CLAUDE_SDK, GEMINI_SDK};
use crate::data::settings::ModelPricing;
use crate::error::{AppError, AppResult};

const LIST_TIMEOUT_SECS: u64 = 30;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Rich model entry returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModelInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_modalities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_modalities: Option<Vec<String>>,
}

/// Fetch the upstream model catalog. Returns a de-duplicated, order-preserving
/// list with as much metadata as the provider exposes.
pub async fn fetch_models(
    sdk: &str,
    endpoint: &str,
    api_key: &str,
) -> AppResult<Vec<RemoteModelInfo>> {
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
    let models = parse_remote_models(&v);
    if models.is_empty() {
        return Err(AppError::Upstream(
            "该供应商未返回任何模型，请检查 API 地址与密钥。".into(),
        ));
    }
    Ok(models)
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

/// Extract rich model entries from OpenRouter / OpenAI / Claude / Gemini shapes.
fn parse_remote_models(v: &Value) -> Vec<RemoteModelInfo> {
    let mut out: Vec<RemoteModelInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for key in ["data", "models"] {
        if let Some(arr) = v.get(key).and_then(Value::as_array) {
            collect_models(arr, &mut out, &mut seen);
        }
    }

    // Some gateways return a bare top-level array.
    if out.is_empty() {
        if let Some(arr) = v.as_array() {
            collect_models(arr, &mut out, &mut seen);
        }
    }

    out
}

fn collect_models(
    arr: &[Value],
    out: &mut Vec<RemoteModelInfo>,
    seen: &mut std::collections::HashSet<String>,
) {
    for item in arr {
        if let Some(info) = parse_one_model(item) {
            if seen.insert(info.id.clone()) {
                out.push(info);
            }
        }
    }
}

fn parse_one_model(item: &Value) -> Option<RemoteModelInfo> {
    // Bare string id.
    if let Some(s) = item.as_str() {
        let id = normalize_model_id(s);
        if id.is_empty() {
            return None;
        }
        return Some(RemoteModelInfo {
            id,
            name: None,
            context_window: None,
            max_output_tokens: None,
            pricing: None,
            capabilities: Vec::new(),
            input_modalities: None,
            output_modalities: None,
        });
    }

    if !item.is_object() {
        return None;
    }

    // Gemini uses `name` like "models/gemini-2.0-flash"; OpenAI/OpenRouter use `id`.
    let raw_id = item
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| item.get("name").and_then(Value::as_str))?;
    let id = normalize_model_id(raw_id);
    if id.is_empty() {
        return None;
    }

    // Prefer human-friendly labels over path-style `name` (Gemini uses models/…).
    let name = ["displayName", "display_name", "name"]
        .iter()
        .find_map(|key| {
            item.get(*key)
                .and_then(Value::as_str)
                .map(|s| s.strip_prefix("models/").unwrap_or(s).trim())
                .filter(|s| !s.is_empty() && *s != id && *s != raw_id)
                .map(str::to_string)
        });

    let context_window = first_i64(
        item,
        &[
            "context_length",
            "context_window",
            "inputTokenLimit",
            "input_token_limit",
        ],
    )
    .or_else(|| {
        item.pointer("/top_provider/context_length")
            .and_then(value_as_i64)
    });

    let max_output_tokens = first_i64(
        item,
        &[
            "max_output_tokens",
            "max_completion_tokens",
            "outputTokenLimit",
            "output_token_limit",
        ],
    )
    .or_else(|| {
        item.pointer("/top_provider/max_completion_tokens")
            .and_then(value_as_i64)
    });

    let pricing = parse_pricing(item.get("pricing"));

    let input_modalities = parse_string_list(
        item
            .pointer("/architecture/input_modalities")
            .or_else(|| item.get("input_modalities")),
    );
    let output_modalities = parse_string_list(
        item
            .pointer("/architecture/output_modalities")
            .or_else(|| item.get("output_modalities")),
    );

    let capabilities = derive_capabilities(item, &input_modalities, &output_modalities);

    Some(RemoteModelInfo {
        id,
        name,
        context_window,
        max_output_tokens,
        pricing,
        capabilities,
        input_modalities,
        output_modalities,
    })
}

fn normalize_model_id(raw: &str) -> String {
    raw.strip_prefix("models/").unwrap_or(raw).trim().to_string()
}

fn first_i64(item: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(n) = item.get(*key).and_then(value_as_i64) {
            if n > 0 {
                return Some(n);
            }
        }
    }
    None
}

fn value_as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_f64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()).map(|n| n as i64))
}

fn parse_string_list(v: Option<&Value>) -> Option<Vec<String>> {
    let arr = v?.as_array()?;
    let out: Vec<String> = arr
        .iter()
        .filter_map(|x| x.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// OpenRouter prices are USD **per token** strings; convert to per-1M.
fn parse_pricing(v: Option<&Value>) -> Option<ModelPricing> {
    let obj = v?.as_object()?;
    let per_1m = |key: &str| -> Option<f64> {
        let raw = obj.get(key)?;
        let n = raw
            .as_f64()
            .or_else(|| raw.as_str().and_then(|s| s.trim().parse::<f64>().ok()))?;
        if !n.is_finite() || n < 0.0 {
            return None;
        }
        // Values already look like per-1M (>= 0.01 and no scientific tiny) keep as-is;
        // OpenRouter classic is ~1e-5–1e-6 per token.
        let per_m = if n > 0.0 && n < 0.01 {
            n * 1_000_000.0
        } else {
            n
        };
        Some(per_m)
    };

    let pricing = ModelPricing {
        input_per_1m: per_1m("prompt").or_else(|| per_1m("input")),
        output_per_1m: per_1m("completion").or_else(|| per_1m("output")),
        cache_read_per_1m: per_1m("input_cache_read").or_else(|| per_1m("cache_read")),
        cache_write_per_1m: per_1m("input_cache_write").or_else(|| per_1m("cache_write")),
    };
    if pricing.is_empty() {
        None
    } else {
        Some(pricing)
    }
}

fn derive_capabilities(
    item: &Value,
    input_modalities: &Option<Vec<String>>,
    output_modalities: &Option<Vec<String>>,
) -> Vec<String> {
    let mut caps = std::collections::BTreeSet::new();

    if let Some(inputs) = input_modalities {
        if inputs.iter().any(|m| m == "image" || m == "vision") {
            caps.insert("vision".into());
        }
        if inputs.iter().any(|m| m == "file" || m == "document") {
            caps.insert("vision".into());
        }
    }
    if let Some(outputs) = output_modalities {
        if outputs.iter().any(|m| m == "image") {
            caps.insert("image".into());
        }
        if outputs.iter().any(|m| m == "video") {
            caps.insert("video".into());
        }
    }

    // OpenRouter modality string e.g. "text+image->text"
    if let Some(mod_str) = item
        .pointer("/architecture/modality")
        .and_then(Value::as_str)
        .map(|s| s.to_ascii_lowercase())
    {
        if mod_str.contains("image") && mod_str.contains("->") {
            let parts: Vec<&str> = mod_str.split("->").collect();
            if parts.first().is_some_and(|p| p.contains("image")) {
                caps.insert("vision".into());
            }
            if parts.get(1).is_some_and(|p| p.contains("image")) {
                caps.insert("image".into());
            }
            if parts.get(1).is_some_and(|p| p.contains("video")) {
                caps.insert("video".into());
            }
        }
    }

    if let Some(params) = item
        .get("supported_parameters")
        .and_then(Value::as_array)
    {
        let joined: Vec<String> = params
            .iter()
            .filter_map(|p| p.as_str())
            .map(|s| s.to_ascii_lowercase())
            .collect();
        if joined.iter().any(|p| p == "tools" || p == "tool_choice") {
            caps.insert("tools".into());
        }
        if joined.iter().any(|p| {
            p.contains("reasoning") || p == "include_reasoning" || p == "thinking"
        }) {
            caps.insert("reasoning".into());
        }
    }

    if item.get("reasoning").is_some() {
        caps.insert("reasoning".into());
    }

    // Gemini supportedGenerationMethods
    if let Some(methods) = item
        .get("supportedGenerationMethods")
        .and_then(Value::as_array)
    {
        if methods.iter().any(|m| {
            m.as_str()
                .map(|s| s.contains("generateContent") || s.contains("generate"))
                .unwrap_or(false)
        }) {
            caps.insert("text".into());
        }
    }

    if caps.is_empty() {
        // Leave empty so the frontend can still apply id heuristics.
        return Vec::new();
    }
    if !caps.contains("image") && !caps.contains("video") && !caps.contains("text") {
        caps.insert("text".into());
    }
    caps.into_iter().collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openrouter_rich_model() {
        let body = json!({
            "data": [{
                "id": "anthropic/claude-opus-5-fast",
                "name": "Claude Opus 5 (Fast)",
                "context_length": 1000000,
                "architecture": {
                    "modality": "text+image+file->text",
                    "input_modalities": ["text", "image", "file"],
                    "output_modalities": ["text"]
                },
                "pricing": {
                    "prompt": "0.00001",
                    "completion": "0.00005",
                    "input_cache_read": "0.000001",
                    "input_cache_write": "0.0000125"
                },
                "top_provider": {
                    "context_length": 1000000,
                    "max_completion_tokens": 128000
                },
                "supported_parameters": ["tools", "reasoning", "max_tokens"],
                "reasoning": { "mandatory": false }
            }]
        });
        let models = parse_remote_models(&body);
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id, "anthropic/claude-opus-5-fast");
        assert_eq!(m.name.as_deref(), Some("Claude Opus 5 (Fast)"));
        assert_eq!(m.context_window, Some(1_000_000));
        assert_eq!(m.max_output_tokens, Some(128_000));
        let p = m.pricing.as_ref().expect("pricing");
        assert!((p.input_per_1m.unwrap() - 10.0).abs() < 1e-9);
        assert!((p.output_per_1m.unwrap() - 50.0).abs() < 1e-9);
        assert!((p.cache_read_per_1m.unwrap() - 1.0).abs() < 1e-9);
        assert!((p.cache_write_per_1m.unwrap() - 12.5).abs() < 1e-9);
        assert!(m.capabilities.iter().any(|c| c == "vision"));
        assert!(m.capabilities.iter().any(|c| c == "tools"));
        assert!(m.capabilities.iter().any(|c| c == "reasoning"));
        assert_eq!(
            m.input_modalities.as_ref().map(|v| v.as_slice()),
            Some(["text".into(), "image".into(), "file".into()].as_slice())
        );
    }

    #[test]
    fn parses_openai_id_only() {
        let body = json!({
            "data": [
                { "id": "gpt-4o" },
                { "id": "o3-mini" }
            ]
        });
        let models = parse_remote_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert!(models[0].context_window.is_none());
        assert!(models[0].pricing.is_none());
    }

    #[test]
    fn parses_gemini_models() {
        let body = json!({
            "models": [{
                "name": "models/gemini-2.0-flash",
                "displayName": "Gemini 2.0 Flash",
                "inputTokenLimit": 1048576,
                "outputTokenLimit": 8192,
                "supportedGenerationMethods": ["generateContent"]
            }]
        });
        let models = parse_remote_models(&body);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-2.0-flash");
        assert_eq!(models[0].context_window, Some(1_048_576));
        assert_eq!(models[0].max_output_tokens, Some(8192));
    }

    #[test]
    fn models_url_strips_chat_suffix() {
        assert_eq!(
            models_url("openai", "https://openrouter.ai/api/v1/chat/completions"),
            "https://openrouter.ai/api/v1/models"
        );
        assert_eq!(
            models_url(
                "openai-responses",
                "https://ark.cn-beijing.volces.com/api/v3/responses"
            ),
            "https://ark.cn-beijing.volces.com/api/v3/models"
        );
    }
}
