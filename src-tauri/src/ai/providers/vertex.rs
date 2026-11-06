//! Google Cloud Vertex AI Gemini (`generateContent` / SSE `streamGenerateContent`).
//!
//! Request JSON and stream parsing are shared with the AI Studio Gemini adapter.
//! This module validates the Vertex URL template and resolves credentials:
//! an API key or access token is passed through, and a service-account JSON
//! is exchanged for a cached OAuth access token.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use super::gemini::{generate_google, generate_google_stream, GoogleAuthStyle};
use super::{ChatProvider, ProviderFuture, VERTEX_SDK};
use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::error::{AppError, AppResult};

const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const ASSERTION_TTL_SECS: i64 = 3600;
const TOKEN_REFRESH_SKEW_SECS: i64 = 60;

pub struct VertexProvider;

impl VertexProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for VertexProvider {
    fn sdk(&self) -> &'static str {
        VERTEX_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move {
            let request = prepare_vertex_request(request).await?;
            generate_google(request, GoogleAuthStyle::VertexAuto).await
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            let request = prepare_vertex_request(request).await?;
            generate_google_stream(request, on_text_delta, GoogleAuthStyle::VertexAuto).await
        })
    }
}

async fn prepare_vertex_request(mut request: ChatRequest) -> AppResult<ChatRequest> {
    ensure_vertex_endpoint_ready(&request.provider.endpoint)?;
    request.provider.api_key = resolve_vertex_credential(&request.provider.api_key).await?;
    Ok(request)
}

/// Vertex default catalog URLs keep `{project}` / `{location}` until the
/// settings UI fills them in. Refuse to send a request that would 404.
pub(crate) fn ensure_vertex_endpoint_ready(endpoint: &str) -> AppResult<()> {
    let e = endpoint.trim();
    if e.is_empty() {
        return Err(AppError::Config("Vertex API 地址不能为空。".into()));
    }
    if e.contains("{project}") || e.contains("{location}") {
        return Err(AppError::Config(
            "Vertex 地址仍含 {project} 或 {location}，请在设置里填写 GCP 项目 ID 和区域。".into(),
        ));
    }
    Ok(())
}

/// API keys and access tokens pass through. A Google service-account JSON
/// is exchanged for an OAuth access token (cached until shortly before expiry).
pub(crate) async fn resolve_vertex_credential(api_key: &str) -> AppResult<String> {
    resolve_vertex_credential_with(api_key, exchange_access_token).await
}

async fn resolve_vertex_credential_with<F, Fut>(api_key: &str, exchange: F) -> AppResult<String>
where
    F: Fn(ServiceAccount) -> Fut,
    Fut: Future<Output = AppResult<ExchangedToken>>,
{
    let Some(account) = parse_service_account_json(api_key)? else {
        return Ok(normalize_secret(api_key).to_string());
    };
    let cache_id = cache_key(&account);
    let now = chrono::Utc::now().timestamp();
    if let Some(token) = cached_token(&cache_id, now) {
        return Ok(token);
    }
    let exchanged = exchange(account).await?;
    store_token(&cache_id, &exchanged, now);
    Ok(exchanged.access_token)
}

struct ServiceAccount {
    client_email: String,
    private_key: String,
    private_key_id: String,
    token_uri: String,
}

struct ExchangedToken {
    access_token: String,
    expires_in: i64,
}

struct CachedToken {
    access_token: String,
    refresh_at_unix: i64,
}

fn token_cache() -> &'static Mutex<HashMap<String, CachedToken>> {
    static CACHE: std::sync::LazyLock<Mutex<HashMap<String, CachedToken>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    &CACHE
}

fn cache_key(account: &ServiceAccount) -> String {
    format!("{}:{}", account.client_email, account.private_key_id)
}

fn cached_token(key: &str, now: i64) -> Option<String> {
    let guard = token_cache().lock().unwrap_or_else(|err| err.into_inner());
    let entry = guard.get(key)?;
    if now >= entry.refresh_at_unix {
        return None;
    }
    Some(entry.access_token.clone())
}

fn store_token(key: &str, token: &ExchangedToken, now: i64) {
    let ttl = token.expires_in.max(1);
    let skew = TOKEN_REFRESH_SKEW_SECS.min(ttl.saturating_sub(1));
    let mut guard = token_cache().lock().unwrap_or_else(|err| err.into_inner());
    guard.insert(
        key.to_string(),
        CachedToken {
            access_token: token.access_token.clone(),
            refresh_at_unix: now + ttl - skew,
        },
    );
}

#[cfg(test)]
fn expire_cached_token(key: &str) {
    let mut guard = token_cache().lock().unwrap_or_else(|err| err.into_inner());
    if let Some(entry) = guard.get_mut(key) {
        entry.refresh_at_unix = 0;
    }
}

fn normalize_secret(raw: &str) -> &str {
    raw.trim().trim_start_matches('\u{feff}').trim()
}

/// `None` when `raw` is not JSON. JSON that is not a usable service account
/// is a config error rather than an API key.
fn parse_service_account_json(raw: &str) -> AppResult<Option<ServiceAccount>> {
    let trimmed = normalize_secret(raw);
    if !trimmed.starts_with('{') {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|_| AppError::Config("Vertex 服务账号 JSON 无法解析。".into()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::Config("Vertex 服务账号 JSON 无法解析。".into()))?;
    let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
    if kind != "service_account" {
        return Err(AppError::Config(
            "Vertex 密钥不是服务账号 JSON（type 须为 service_account）。".into(),
        ));
    }
    let client_email = obj
        .get("client_email")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if client_email.is_empty() {
        return Err(AppError::Config(
            "Vertex 服务账号 JSON 缺少 client_email。".into(),
        ));
    }
    let private_key = obj.get("private_key").and_then(Value::as_str).unwrap_or("");
    if !private_key.contains("BEGIN PRIVATE KEY") && !private_key.contains("BEGIN RSA PRIVATE KEY")
    {
        return Err(AppError::Config(
            "Vertex 服务账号 JSON 缺少 private_key。".into(),
        ));
    }
    let token_uri = match obj.get("token_uri").and_then(Value::as_str) {
        None => TOKEN_URI.to_string(),
        Some(uri) => {
            let uri = uri.trim();
            if uri.is_empty() {
                TOKEN_URI.to_string()
            } else if uri != TOKEN_URI {
                return Err(AppError::Config(
                    "Vertex 服务账号 token_uri 仅支持 https://oauth2.googleapis.com/token。".into(),
                ));
            } else {
                uri.to_string()
            }
        }
    };
    let private_key_id = obj
        .get("private_key_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(Some(ServiceAccount {
        client_email: client_email.to_string(),
        private_key: private_key.to_string(),
        private_key_id,
        token_uri,
    }))
}

#[derive(Serialize)]
struct AssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

fn sign_assertion(account: &ServiceAccount, now: i64) -> AppResult<String> {
    let claims = AssertionClaims {
        iss: &account.client_email,
        scope: CLOUD_PLATFORM_SCOPE,
        aud: &account.token_uri,
        iat: now,
        exp: now + ASSERTION_TTL_SECS,
    };
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(account.private_key.as_bytes())
        .map_err(|_| AppError::Config("Vertex 服务账号私钥无法用于签名。".into()))?;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &key,
    )
    .map_err(|_| AppError::Config("Vertex 服务账号私钥无法用于签名。".into()))
}

async fn exchange_access_token(account: ServiceAccount) -> AppResult<ExchangedToken> {
    let assertion = sign_assertion(&account, chrono::Utc::now().timestamp())?;
    let body = format!(
        "grant_type={}&assertion={}",
        form_encode("urn:ietf:params:oauth:grant-type:jwt-bearer"),
        form_encode(&assertion),
    );
    let client = crate::ai::providers::build_chat_client()?;
    let resp = client
        .post(&account.token_uri)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "Vertex 服务账号换取 access token 失败 HTTP {status}: {}",
            token_error_message(&text)
        )));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|_| {
        AppError::Upstream("Vertex 服务账号换取 access token 的响应无法解析。".into())
    })?;
    let access_token = parsed
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if access_token.is_empty() {
        return Err(AppError::Upstream(
            "Vertex 服务账号换取 access token 的响应缺少 access_token。".into(),
        ));
    }
    let expires_in = parsed
        .get("expires_in")
        .and_then(Value::as_i64)
        .filter(|secs| *secs > 0)
        .unwrap_or(ASSERTION_TTL_SECS);
    Ok(ExchangedToken {
        access_token,
        expires_in,
    })
}

fn token_error_message(text: &str) -> String {
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let description = value
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let code = value.get("error").and_then(Value::as_str).unwrap_or("");
        let message = if !description.is_empty() {
            description
        } else {
            code
        };
        if !message.is_empty() {
            return truncate_chars(message, 300);
        }
    }
    if trimmed.is_empty() {
        return "空响应".into();
    }
    truncate_chars(trimmed, 300)
}

fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn truncate_chars(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::ai::providers::gemini::gemini_url;
    use serde_json::json;

    #[test]
    fn leftover_placeholders_are_config_errors() {
        let err = ensure_vertex_endpoint_ready(
            "https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent",
        )
        .unwrap_err();
        match err {
            AppError::Config(msg) => {
                assert!(msg.contains("{project}") || msg.contains("{location}"));
            }
            other => panic!("expected Config, got {other:?}"),
        }
        let err = ensure_vertex_endpoint_ready(
            "https://aiplatform.googleapis.com/v1/projects/{project}/locations/global/publishers/google/models/{model}:generateContent",
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn filled_vertex_url_is_ready() {
        ensure_vertex_endpoint_ready(
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/{model}:generateContent",
        )
        .unwrap();
    }

    #[test]
    fn empty_endpoint_is_config_error() {
        assert!(matches!(
            ensure_vertex_endpoint_ready("  "),
            Err(AppError::Config(_))
        ));
    }

    #[test]
    fn global_and_regional_stream_urls() {
        let global = "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(global, "gemini-2.5-flash", true),
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        let regional = "https://europe-west1-aiplatform.googleapis.com/v1/projects/my-proj/locations/europe-west1/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(regional, "gemini-2.5-pro", false),
            "https://europe-west1-aiplatform.googleapis.com/v1/projects/my-proj/locations/europe-west1/publishers/google/models/gemini-2.5-pro:generateContent"
        );
    }

    const DUMMY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----";

    fn config_message(result: AppResult<Option<ServiceAccount>>) -> String {
        match result {
            Err(AppError::Config(msg)) => msg,
            Ok(_) => panic!("expected a config error"),
            Err(other) => panic!("expected a config error, got {other}"),
        }
    }

    fn sa_json(email: &str, key_id: &str, pem: &str, token_uri: Option<&str>) -> String {
        let mut value = json!({
            "type": "service_account",
            "client_email": email,
            "private_key": pem,
            "private_key_id": key_id,
        });
        if let Some(uri) = token_uri {
            value["token_uri"] = json!(uri);
        }
        value.to_string()
    }

    #[test]
    fn service_account_json_requires_email_key_and_official_token_uri() {
        let ok = parse_service_account_json(&sa_json(
            "bot@example.iam.gserviceaccount.com",
            "kid",
            DUMMY_PEM,
            None,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(ok.client_email, "bot@example.iam.gserviceaccount.com");
        assert_eq!(ok.token_uri, TOKEN_URI);

        let err = config_message(parse_service_account_json("{"));
        assert!(err.contains("无法解析"));

        let err = config_message(parse_service_account_json(r#"{"type":"service_account"}"#));
        assert!(err.contains("client_email"));

        let err = config_message(parse_service_account_json(&sa_json(
            "bot@example.iam.gserviceaccount.com",
            "kid",
            DUMMY_PEM,
            Some("https://evil.example/token"),
        )));
        assert!(err.contains("token_uri"));
    }

    #[test]
    fn service_account_assertion_is_rs256_with_cloud_platform_scope() {
        let email = "signer@example.iam.gserviceaccount.com";
        let raw = sa_json(
            email,
            "kid",
            include_str!("vertex_test_key.pem"),
            Some(TOKEN_URI),
        );
        let account = parse_service_account_json(&raw).unwrap().unwrap();
        let now = 1_700_000_000;
        let token = sign_assertion(&account, now).unwrap();

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_audience(&[TOKEN_URI]);
        validation.set_issuer(&[email]);
        validation.validate_exp = false;
        let key =
            jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("vertex_test_pub.pem")).unwrap();
        let data = jsonwebtoken::decode::<serde_json::Value>(&token, &key, &validation).unwrap();
        assert_eq!(data.header.alg, jsonwebtoken::Algorithm::RS256);
        assert_eq!(data.claims["iss"], email);
        assert_eq!(data.claims["scope"], CLOUD_PLATFORM_SCOPE);
        assert_eq!(data.claims["aud"], TOKEN_URI);
        assert_eq!(data.claims["iat"], now);
        assert_eq!(data.claims["exp"], now + ASSERTION_TTL_SECS);
    }

    #[tokio::test]
    async fn plain_api_key_is_not_exchanged() {
        let exchange = |_account: ServiceAccount| async move {
            Err::<ExchangedToken, _>(AppError::Config("exchange should not run".into()))
        };
        let out = resolve_vertex_credential_with("  AIzaSyKey  ", exchange)
            .await
            .unwrap();
        assert_eq!(out, "AIzaSyKey");
        assert!(parse_service_account_json("ya29.token").unwrap().is_none());
    }

    #[tokio::test]
    async fn service_account_token_is_cached_until_refresh() {
        let email = "cache-vertex@example.iam.gserviceaccount.com";
        let raw = sa_json(email, "cache-key", DUMMY_PEM, None);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_exchange = calls.clone();
        let exchange = move |_account: ServiceAccount| {
            let calls_for_exchange = calls_for_exchange.clone();
            async move {
                calls_for_exchange.fetch_add(1, Ordering::SeqCst);
                Ok(ExchangedToken {
                    access_token: "ya29.cached-token".into(),
                    expires_in: 3600,
                })
            }
        };

        let first = resolve_vertex_credential_with(&raw, &exchange)
            .await
            .unwrap();
        let second = resolve_vertex_credential_with(&raw, &exchange)
            .await
            .unwrap();
        assert_eq!(first, "ya29.cached-token");
        assert_eq!(second, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let account = parse_service_account_json(&raw).unwrap().unwrap();
        expire_cached_token(&cache_key(&account));
        let third = resolve_vertex_credential_with(&raw, &exchange)
            .await
            .unwrap();
        assert_eq!(third, first);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
