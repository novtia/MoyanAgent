use crate::ai::chat::ChatRequest;
use crate::error::AppError;

use super::body::responses_cache_active;

pub(crate) fn should_reset_responses_cache(err: &AppError, request: &ChatRequest) -> bool {
    if !responses_cache_active(request)
        || request
            .previous_response_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none()
    {
        return false;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("previous_response")
        || msg.contains("response_id")
        || msg.contains("caching")
        || msg.contains("context cache")
        || msg.contains("not found")
        || msg.contains("expired")
        || msg.contains("invalid")
}

/// True when the upstream rejected Session caching for this model/account
/// (e.g. Ark: "has not activated the cache service for model …"). Retry
/// once with caching disabled rather than failing the whole turn.
pub(crate) fn should_disable_responses_cache(err: &AppError, request: &ChatRequest) -> bool {
    if !responses_cache_active(request) {
        return false;
    }
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("has not activated the cache")
        || msg.contains("activate the cache service")
        || (msg.contains("cache service") && msg.contains("403"))
        || msg.contains("caching is not supported")
        || msg.contains("does not support caching")
}
/// Best-effort DELETE of a stored Responses API object (Session cache tip).
/// Failures are logged and ignored — local DB is the source of truth for clearing.
pub async fn delete_stored_response(endpoint: &str, api_key: &str, response_id: &str) {
    let id = response_id.trim();
    if id.is_empty() || api_key.trim().is_empty() {
        return;
    }
    let Some(url) = responses_object_url(endpoint, id) else {
        return;
    };
    let Ok(client) = crate::ai::providers::build_chat_client() else {
        return;
    };
    match client.delete(&url).bearer_auth(api_key).send().await {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => {}
        Ok(resp) => {
            eprintln!(
                "[atelier] delete stored response {} failed: HTTP {}",
                id,
                resp.status()
            );
        }
        Err(err) => {
            eprintln!("[atelier] delete stored response {id} failed: {err}");
        }
    }
}

pub(crate) fn responses_object_url(endpoint: &str, response_id: &str) -> Option<String> {
    let ep = endpoint.trim().trim_end_matches('/');
    if ep.is_empty() {
        return None;
    }
    // Typical: .../api/v3/responses  or  .../v1/responses
    if ep.ends_with("/responses") {
        return Some(format!("{ep}/{response_id}"));
    }
    // .../chat/completions → sibling /responses/{id}
    if let Some(base) = ep.strip_suffix("/chat/completions") {
        return Some(format!("{base}/responses/{response_id}"));
    }
    // Bare API root: .../api/v3
    Some(format!("{ep}/responses/{response_id}"))
}
