use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::{json, Map, Value};

use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::error::{AppError, AppResult};

use super::chat::parse::parse_openai_like_response;
use super::chat::stream::parse_openai_chat_success;
use super::common::{
    debug_log_upstream_request, debug_log_upstream_response_text, emit_final_text_if_needed,
    is_empty_stream_upstream_error, is_idle_timeout_message, message_rejects_gemini_arg_streaming,
    retryable_error_in_json_body, should_retry_failed_stream_attempt, should_retry_http_error,
    should_retry_transport, sleep_for_attempt, upstream_error_message,
    upstream_rejects_gemini_arg_streaming, upstream_rejects_streaming, without_streaming,
    MAX_ATTEMPTS,
};

pub(crate) async fn post_openrouter_chat(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &mut Value,
    provider_label: &str,
) -> AppResult<String> {
    let mut modality_stage = openrouter_initial_modality_stage(request);
    let mut tools_stripped = false;
    'modalities: loop {
        apply_openrouter_modalities_stage(body, &request.model, modality_stage);

        for attempt in 1..=MAX_ATTEMPTS {
            if attempt == 1 {
                debug_log_upstream_request(provider_label, &request.provider.endpoint, body);
            }
            let resp = client
                .post(&request.provider.endpoint)
                .bearer_auth(&request.provider.api_key)
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
            let txt = match resp.text().await {
                Ok(txt) => txt,
                Err(err) => {
                    if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                        sleep_for_attempt(attempt).await;
                        continue;
                    }
                    return Err(err.into());
                }
            };
            if status.is_success() {
                if attempt < MAX_ATTEMPTS && retryable_error_in_json_body(&txt) {
                    prepare_openrouter_retry(body, &upstream_error_message(&txt));
                    sleep_for_attempt(attempt).await;
                    continue;
                }
                debug_log_upstream_response_text(provider_label, &txt);
                return Ok(txt);
            }

            let msg = upstream_error_message(&txt);
            if attempt < MAX_ATTEMPTS && should_retry_http_error(status, &msg) {
                prepare_openrouter_retry(body, &msg);
                sleep_for_attempt(attempt).await;
                continue;
            }
            if upstream_rejects_tools(status, &msg) && !tools_stripped {
                tools_stripped = true;
                strip_tools_from_body(body);
                continue 'modalities;
            }
            if upstream_rejects_modalities(status, &msg) && modality_stage < 2 {
                modality_stage += 1;
                continue 'modalities;
            }
            return Err(AppError::Upstream(format!(
                "{} HTTP {}: {}",
                provider_label, status, msg
            )));
        }
        unreachable!("HTTP attempts should return or branch before completing the loop");
    }
}

pub(crate) async fn post_openrouter_chat_stream(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &mut Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut modality_stage = openrouter_initial_modality_stage(request);
    let mut tools_stripped = false;
    let mut gemini_arg_stream_stripped = false;
    'modalities: loop {
        apply_openrouter_modalities_stage(body, &request.model, modality_stage);

        for attempt in 1..=MAX_ATTEMPTS {
            if attempt == 1 {
                debug_log_upstream_request(provider_label, &request.provider.endpoint, body);
            }
            let resp = client
                .post(&request.provider.endpoint)
                .bearer_auth(&request.provider.api_key)
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
            if status.is_success() {
                // Only retry a transient transport failure while no delta was
                // emitted yet, otherwise the user would see the response
                // restart.
                let emitted = Arc::new(AtomicBool::new(false));
                let flag = emitted.clone();
                let base = on_text_delta.clone();
                let tracked: TextDeltaCallback = Arc::new(move |delta| {
                    flag.store(true, Ordering::Relaxed);
                    (base)(delta);
                });
                return match parse_openai_chat_success(resp, tracked).await {
                    Ok(r) => Ok(r),
                    Err(e) if is_empty_stream_upstream_error(&e) => {
                        fallback_openrouter_chat_response(
                            client,
                            request,
                            body,
                            provider_label,
                            on_text_delta.clone(),
                        )
                        .await
                    }
                    Err(e)
                        if should_retry_failed_stream_attempt(
                            &e,
                            attempt,
                            emitted.load(Ordering::Relaxed),
                        ) =>
                    {
                        prepare_openrouter_retry(body, app_error_message(&e));
                        sleep_for_attempt(attempt).await;
                        continue;
                    }
                    Err(e) => Err(e),
                };
            }

            let txt = match resp.text().await {
                Ok(txt) => txt,
                Err(err) => {
                    if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                        sleep_for_attempt(attempt).await;
                        continue;
                    }
                    return Err(err.into());
                }
            };
            let msg = upstream_error_message(&txt);
            if attempt < MAX_ATTEMPTS && should_retry_http_error(status, &msg) {
                prepare_openrouter_retry(body, &msg);
                sleep_for_attempt(attempt).await;
                continue;
            }
            if upstream_rejects_streaming(status, &msg) {
                return fallback_openrouter_chat_response(
                    client,
                    request,
                    body,
                    provider_label,
                    on_text_delta.clone(),
                )
                .await;
            }
            if upstream_rejects_gemini_arg_streaming(status, &msg) && !gemini_arg_stream_stripped {
                gemini_arg_stream_stripped = true;
                strip_gemini_stream_function_call_config(body);
                continue 'modalities;
            }
            if upstream_rejects_tools(status, &msg) && !tools_stripped {
                tools_stripped = true;
                strip_tools_from_body(body);
                continue 'modalities;
            }
            if upstream_rejects_modalities(status, &msg) && modality_stage < 2 {
                modality_stage += 1;
                continue 'modalities;
            }
            return Err(AppError::Upstream(format!(
                "{} HTTP {}: {}",
                provider_label, status, msg
            )));
        }
        unreachable!("HTTP attempts should return or branch before completing the loop");
    }
}
pub(crate) async fn fallback_openrouter_chat_response(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
    on_text_delta: TextDeltaCallback,
) -> AppResult<GenerateResponse> {
    let mut body = without_streaming(body);
    let final_txt = post_openrouter_chat(client, request, &mut body, provider_label).await?;
    let resp = parse_openai_like_response(&final_txt)?;
    emit_final_text_if_needed(&resp, &on_text_delta);
    Ok(resp)
}
pub(crate) fn is_openrouter_endpoint(endpoint: &str) -> bool {
    endpoint
        .trim()
        .to_ascii_lowercase()
        .contains("openrouter.ai")
}

/// Pin the request to specific OpenRouter upstreams via `provider.only`.
pub(crate) fn apply_openrouter_provider_routing(
    map: &mut Map<String, Value>,
    request: &ChatRequest,
) {
    if !is_openrouter_endpoint(&request.provider.endpoint) {
        return;
    }
    let only: Vec<Value> = request
        .route_providers
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| json!(s))
        .collect();
    if only.is_empty() {
        return;
    }
    map.insert("provider".into(), json!({ "only": only }));
}

fn app_error_message(err: &AppError) -> &str {
    match err {
        AppError::Http(s) | AppError::Upstream(s) | AppError::Other(s) => s,
        _ => "",
    }
}

/// After a transient OpenRouter failure: drop `provider.only` so the next
/// attempt can land on a different host. On idle timeout, also step down
/// reasoning effort — silent Gemini-3 thinking is what trips the ~150s
/// gateway idle cap on long post-tool writing turns.
pub(crate) fn prepare_openrouter_retry(body: &mut Value, msg: &str) {
    relax_openrouter_provider_routing(body);
    if is_idle_timeout_message(msg) {
        step_down_openrouter_reasoning(body);
    }
}

/// Drop `provider.only` after a 502/503/504 (especially OpenRouter idle
/// timeout). Retrying the same pin usually lands on the upstream that already
/// hung; without the pin OpenRouter can pick a different one.
pub(crate) fn relax_openrouter_provider_routing(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("provider");
    }
}

pub(crate) fn step_down_openrouter_reasoning(body: &mut Value) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    let current = map
        .get("reasoning")
        .and_then(|v| v.get("effort"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let next = match current.as_deref() {
        Some("max") | Some("xhigh") | Some("high") | Some("medium") => "low",
        _ => "none",
    };
    if next == "none" {
        map.insert(
            "reasoning".into(),
            json!({ "enabled": false, "effort": "none", "exclude": false }),
        );
    } else {
        map.insert(
            "reasoning".into(),
            json!({ "enabled": true, "effort": next, "exclude": false }),
        );
    }
}

/// Agent tool-calling requests must not ask OpenRouter for image output.
pub(crate) fn openrouter_wants_image_output(request: &ChatRequest) -> bool {
    if !request.tools.is_empty() {
        return false;
    }
    if is_image_only_model(&request.model) {
        return true;
    }
    if request.parameters.image_config().is_some() {
        return true;
    }
    request.model.trim().to_ascii_lowercase().contains("image")
}

pub(crate) fn openrouter_initial_modality_stage(request: &ChatRequest) -> u8 {
    if openrouter_wants_image_output(request) {
        0
    } else {
        2
    }
}

pub(crate) fn requested_modalities(model: &str) -> Value {
    if is_image_only_model(model) {
        json!(["image"])
    } else {
        json!(["image", "text"])
    }
}

pub(crate) fn apply_openrouter_modalities_stage(body: &mut Value, model: &str, stage: u8) {
    let Some(map) = body.as_object_mut() else {
        return;
    };
    match stage {
        0 => {
            map.insert("modalities".into(), requested_modalities(model));
        }
        1 => {
            map.insert("modalities".into(), json!(["image"]));
        }
        _ => {
            map.remove("modalities");
        }
    }
}

pub(crate) fn upstream_rejects_modalities(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    (status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST) && m.contains("modalit")
}

/// OpenRouter returns HTTP 404 with a message like
/// "No endpoints found that support tool use." when the selected
/// model/provider combination doesn't support function calling. We
/// detect this and retry without `tools` so the agent can still
/// produce a plain-text response rather than surfacing a hard error.
pub(crate) fn upstream_rejects_tools(status: StatusCode, msg: &str) -> bool {
    // A 400 about `streamFunctionCallArguments` / `toolConfig` is not "this
    // model has no tools" — stripping `tools` would silently disable CreateDoc.
    if message_rejects_gemini_arg_streaming(msg) {
        return false;
    }
    let m = msg.to_ascii_lowercase();
    matches!(status, StatusCode::NOT_FOUND | StatusCode::BAD_REQUEST)
        && (m.contains("tool use")
            || m.contains("tool_use")
            || m.contains("function call")
            || m.contains("invalid_argument"))
}

pub(crate) fn strip_tools_from_body(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("tools");
        map.remove("tool_choice");
    }
}

pub(crate) fn is_gemini_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().contains("gemini")
}

/// Ask Gemini (via OpenRouter) to stream function-call argument fragments
/// instead of buffering the whole JSON. Harmless on non-Gemini models; some
/// upstreams 400 and the stream loop strips this and retries.
pub(crate) fn apply_openrouter_gemini_arg_streaming(body: &mut Value, request: &ChatRequest) {
    if !is_openrouter_endpoint(&request.provider.endpoint) {
        return;
    }
    if request.tools.is_empty() || !is_gemini_model(&request.model) {
        return;
    }
    let Some(map) = body.as_object_mut() else {
        return;
    };
    map.insert(
        "toolConfig".into(),
        json!({
            "functionCallingConfig": {
                "streamFunctionCallArguments": true
            }
        }),
    );
}

pub(crate) fn strip_gemini_stream_function_call_config(body: &mut Value) {
    if let Some(map) = body.as_object_mut() {
        map.remove("toolConfig");
        map.remove("tool_config");
    }
}
pub(crate) fn is_image_only_model(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    m.starts_with("black-forest-labs/")
        || m.starts_with("bytedance-seed/")
        || m.starts_with("sourceful/")
        || m.starts_with("recraft/")
}
