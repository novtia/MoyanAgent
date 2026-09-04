use reqwest::StatusCode;
use serde_json::Value;

use crate::ai::chat::ChatRequest;
use crate::error::{AppError, AppResult};

use super::debug::{debug_log_upstream_request, debug_log_upstream_response_text};
use super::error::{
    is_retryable_upstream_message, retryable_error_in_json_body, upstream_error_message,
};

pub(crate) const MAX_ATTEMPTS: usize = 3;
pub(crate) async fn post_with_retries(
    client: &reqwest::Client,
    request: &ChatRequest,
    body: &Value,
    provider_label: &str,
) -> AppResult<String> {
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
                // Reading the body can fail on an abrupt connection close
                // (e.g. "peer closed connection without sending TLS
                // close_notify") — transient, so retry like a send failure.
                if attempt < MAX_ATTEMPTS && should_retry_transport(&err) {
                    sleep_for_attempt(attempt).await;
                    continue;
                }
                return Err(err.into());
            }
        };
        if status.is_success() {
            // OpenRouter (and some other gateways) return HTTP 200 with
            // `{error:{code:504,message:"Upstream idle timeout exceeded"}}`
            // instead of a 504 status. Treat those like a retryable 504.
            if attempt < MAX_ATTEMPTS && retryable_error_in_json_body(&txt) {
                sleep_for_attempt(attempt).await;
                continue;
            }
            debug_log_upstream_response_text(provider_label, &txt);
            return Ok(txt);
        }

        let msg = upstream_error_message(&txt);
        if attempt < MAX_ATTEMPTS && should_retry_http_error(status, &msg) {
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
pub(crate) fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
    )
}

pub(crate) fn should_retry_http_error(status: StatusCode, msg: &str) -> bool {
    is_retryable_status(status) || is_retryable_upstream_message(msg)
}

pub(crate) fn should_retry_transport(err: &reqwest::Error) -> bool {
    err.is_timeout()
        || err.is_connect()
        || err.is_request()
        || crate::error::reqwest_error_indicates_abrupt_close(err)
}

pub(crate) async fn sleep_for_attempt(attempt: usize) {
    let backoff_ms = 500u64 * (1u64 << (attempt - 1));
    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
}
