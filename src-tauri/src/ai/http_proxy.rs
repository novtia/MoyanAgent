//! Process-wide HTTP/SOCKS proxy used by every outbound `reqwest` client.
//!
//! The URL is loaded from settings on startup and refreshed whenever the user
//! saves the System page. Clients are built per request, so a change applies
//! to the next call with no restart.

use std::sync::RwLock;

use crate::data::settings::Settings;
use crate::error::{AppError, AppResult};

static HTTP_PROXY: RwLock<Option<String>> = RwLock::new(None);

const ALLOWED_SCHEMES: &[&str] = &["http", "https", "socks5", "socks5h"];

/// Normalize and accept a user-supplied proxy URL.
pub fn validate_proxy_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("代理地址不能为空".into()));
    }
    let url = reqwest::Url::parse(trimmed)
        .map_err(|e| AppError::Invalid(format!("代理地址无效：{e}")))?;
    if !ALLOWED_SCHEMES.contains(&url.scheme()) {
        return Err(AppError::Invalid(format!(
            "代理协议仅支持 http、https、socks5、socks5h，收到 `{}`",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(AppError::Invalid("代理地址必须包含主机".into()));
    }
    Ok(trimmed.to_string())
}

pub fn set_from_settings(settings: &Settings) {
    let next = if settings.http_proxy_enabled {
        validate_proxy_url(&settings.http_proxy_url).ok()
    } else {
        None
    };
    *HTTP_PROXY.write().unwrap_or_else(|e| e.into_inner()) = next;
}

pub fn current_url() -> Option<String> {
    HTTP_PROXY.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn is_enabled() -> bool {
    current_url().is_some()
}

/// Attach the configured proxy (if any). When a proxy is set, environment
/// proxies are ignored so the two cannot stack.
pub fn apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::Result<reqwest::ClientBuilder> {
    match current_url() {
        Some(url) => Ok(builder.no_proxy().proxy(reqwest::Proxy::all(url)?)),
        None => Ok(builder),
    }
}

pub fn build_client(builder: reqwest::ClientBuilder) -> reqwest::Result<reqwest::Client> {
    apply_proxy(builder)?.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_socks_urls() {
        for raw in [
            "http://127.0.0.1:7890",
            "https://proxy.example:8443",
            "socks5://127.0.0.1:7891",
            "socks5h://127.0.0.1:1080",
            "http://user:pass@127.0.0.1:6867",
            "  http://127.0.0.1:6867  ",
        ] {
            assert!(validate_proxy_url(raw).is_ok(), "`{raw}` should be valid");
        }
    }

    #[test]
    fn rejects_empty_and_unsupported_schemes() {
        for raw in [
            "",
            "   ",
            "ftp://127.0.0.1:21",
            "file:///tmp/proxy",
            "127.0.0.1:7890",
            "http://",
        ] {
            assert!(
                validate_proxy_url(raw).is_err(),
                "`{raw}` should be rejected"
            );
        }
    }
}
