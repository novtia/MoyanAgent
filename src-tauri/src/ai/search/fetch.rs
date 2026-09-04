//! Fetch a single web page and convert it to readable Markdown.
//!
//! Uses `html-to-markdown-rs` with aggressive preprocessing to strip
//! navigation, forms, and other boilerplate before conversion.
//!
//! The URL comes from the model, so every request is treated as attacker-chosen
//! and checked before it is made: only `http(s)`, only public IP addresses, and
//! every redirect hop re-validated. Without that, a page the model was merely
//! asked to summarize can reach `http://127.0.0.1:11434`, a router admin panel,
//! or a cloud instance-metadata endpoint and hand the response back as page
//! text.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use html_to_markdown_rs::{convert, ConversionOptions, PreprocessingOptions, PreprocessingPreset};
use reqwest::Url;

use crate::ai::search::USER_AGENT;
use crate::error::{AppError, AppResult};

/// Hard cap on extracted text so a huge page can't blow up a tool result.
const MAX_TEXT_CHARS: usize = 24_000;
/// Hard cap on bytes read off the wire. `Response::text()` is unbounded, so a
/// hostile or merely enormous URL could otherwise exhaust memory long before
/// the text cap above ever applies.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
/// Redirects are followed manually so each hop can be re-validated; this is the
/// budget for that walk.
const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const REQUEST_TIMEOUT_SECS: u64 = 30;

pub struct FetchedPage {
    pub url: String,
    pub title: String,
    pub text: String,
    /// True when the text was truncated to [`MAX_TEXT_CHARS`].
    pub truncated: bool,
}

pub async fn fetch_page(url: &str) -> AppResult<FetchedPage> {
    let mut target = parse_fetchable_url(url)?;
    let mut hops = 0usize;

    let (final_url, body) = loop {
        // Resolve and vet the destination ourselves, then pin the address we
        // approved for the actual request. Re-resolving inside the HTTP client
        // would reopen the door to a DNS answer that changes between the check
        // and the connection (DNS rebinding).
        let addr = resolve_public_addr(&target).await?;
        let client = build_pinned_client(&target, addr)?;

        let resp = client
            .get(target.clone())
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await?;
        let status = resp.status();

        if status.is_redirection() {
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(AppError::Http(format!(
                    "fetch gave up after {MAX_REDIRECTS} redirects"
                )));
            }
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    AppError::Http(format!(
                        "fetch got HTTP {} with no Location",
                        status.as_u16()
                    ))
                })?;
            // Relative redirects are legal, hence `join` rather than a re-parse.
            let next = target.join(location).map_err(|e| {
                AppError::Invalid(format!("fetch: invalid redirect target `{location}`: {e}"))
            })?;
            target = check_url_scheme(next)?;
            continue;
        }

        if !status.is_success() {
            return Err(AppError::Http(format!(
                "fetch returned HTTP {}",
                status.as_u16()
            )));
        }
        ensure_textual(&resp)?;
        break (target.to_string(), read_capped(resp).await?);
    };

    let (title, mut text) = extract(&body)?;
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    if truncated {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
    }
    Ok(FetchedPage {
        url: final_url,
        title,
        text,
        truncated,
    })
}

/// Parse a model-supplied URL, rejecting everything that is not a plain
/// http(s) request for a named host.
fn parse_fetchable_url(raw: &str) -> AppResult<Url> {
    let parsed = Url::parse(raw.trim())
        .map_err(|e| AppError::Invalid(format!("url must be an absolute http(s) URL: {e}")))?;
    check_url_scheme(parsed)
}

fn check_url_scheme(url: Url) -> AppResult<Url> {
    match url.scheme() {
        "http" | "https" => {}
        // `file:` would read local disk, and `gopher:`/`ftp:` are classic
        // request-smuggling vectors.
        other => {
            return Err(AppError::Invalid(format!(
                "url must be http or https, got `{other}`"
            )));
        }
    }
    // Embedded credentials are only ever used to disguise the real host
    // (`https://trusted.com@attacker.test`).
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Invalid(
            "url must not contain embedded credentials".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(AppError::Invalid("url must have a host".into()));
    }
    Ok(url)
}

/// Resolve `url`'s host and return the one address that may be contacted.
///
/// Every resolved address must be public: a host that answers with a mix of
/// public and private addresses is rejected outright rather than filtered, so
/// nothing depends on which entry the connector would have picked.
async fn resolve_public_addr(url: &Url) -> AppResult<SocketAddr> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Invalid("url must have a host".into()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AppError::Invalid("url has no usable port".into()))?;

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| AppError::Http(format!("fetch: cannot resolve `{host}`: {e}")))?
        .collect();

    let first = *addrs
        .first()
        .ok_or_else(|| AppError::Http(format!("fetch: `{host}` resolved to no address")))?;
    for addr in &addrs {
        if !ip_is_public(addr.ip()) {
            return Err(AppError::Invalid(format!(
                "fetch: `{host}` resolves to the non-public address {} — refusing to fetch \
                 internal network resources",
                addr.ip()
            )));
        }
    }
    Ok(first)
}

/// A client that talks only to `addr`, with redirects disabled so the caller
/// can vet each hop.
fn build_pinned_client(url: &Url, addr: SocketAddr) -> AppResult<reqwest::Client> {
    let host = url.host_str().unwrap_or_default();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(host, addr)
        .build()
        .map_err(|e| AppError::Http(format!("fetch: cannot build client: {e}")))
}

/// Refuse content that is not text before spending bandwidth on it.
fn ensure_textual(resp: &reqwest::Response) -> AppResult<()> {
    check_content_type(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
    )
}

fn check_content_type(declared: Option<&str>) -> AppResult<()> {
    let Some(ctype) = declared else {
        // No declaration: let the HTML extractor decide.
        return Ok(());
    };
    let mime = ctype
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let textual = mime.starts_with("text/")
        || mime.ends_with("+xml")
        || mime.ends_with("+json")
        || matches!(
            mime.as_str(),
            "application/json" | "application/xml" | "application/xhtml+xml" | ""
        );
    if textual {
        return Ok(());
    }
    Err(AppError::Invalid(format!(
        "fetch: `{mime}` is not a readable page — WebFetch only returns text content"
    )))
}

/// Read the body, stopping at [`MAX_BODY_BYTES`].
async fn read_capped(mut resp: reqwest::Response) -> AppResult<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() >= MAX_BODY_BYTES {
            break;
        }
        let take = chunk.len().min(MAX_BODY_BYTES - buf.len());
        buf.extend_from_slice(&chunk[..take]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Whether `ip` is a routable public address.
///
/// Written out rather than using the standard library's `is_global`, which is
/// still unstable. Anything not positively known to be public is refused: the
/// cost of wrongly refusing a page is a failed tool call, while the cost of
/// wrongly allowing one is a request to the user's own network.
fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_public(v4),
        IpAddr::V6(v6) => ipv6_is_public(v6),
    }
}

fn ipv4_is_public(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    if ip.is_private()          // 10/8, 172.16/12, 192.168/16
        || ip.is_loopback()     // 127/8
        || ip.is_link_local()   // 169.254/16 — cloud metadata lives here
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return false;
    }
    match (a, b) {
        (0, _) => false,             // 0.0.0.0/8 "this network"
        (100, 64..=127) => false,    // carrier-grade NAT
        (192, 0) if c == 0 => false, // IETF protocol assignments
        (198, 18 | 19) => false,     // benchmarking
        (240..=255, _) => false,     // reserved / limited broadcast
        _ => true,
    }
}

fn ipv6_is_public(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // An address carrying a v4 destination is only as public as that v4 is:
    // `::ffff:127.0.0.1` and the NAT64 prefix both reach IPv4 loopback.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return ipv4_is_public(v4);
    }
    let seg = ip.segments();
    if seg[0] == 0x0064 && seg[1] == 0xff9b {
        let v4 = Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            (seg[6] & 0xff) as u8,
            (seg[7] >> 8) as u8,
            (seg[7] & 0xff) as u8,
        );
        return ipv4_is_public(v4);
    }
    if seg[0] & 0xfe00 == 0xfc00 {
        return false; // fc00::/7 unique local
    }
    if seg[0] & 0xffc0 == 0xfe80 {
        return false; // fe80::/10 link local
    }
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return false; // documentation
    }
    true
}

fn extract(html: &str) -> AppResult<(String, String)> {
    let mut options = ConversionOptions::default();
    options.preprocessing = PreprocessingOptions {
        enabled: true,
        preset: PreprocessingPreset::Aggressive,
        remove_navigation: true,
        remove_forms: true,
    };
    options.skip_images = true;
    options.extract_metadata = true;

    let result = convert(html, options)
        .map_err(|e| AppError::Other(format!("html to markdown conversion failed: {e}")))?;

    let title = result.metadata.document.title.unwrap_or_default();
    let text = result.content.unwrap_or_default();
    Ok((title, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_urls_for_named_hosts_are_accepted() {
        assert!(parse_fetchable_url("https://example.com/a").is_ok());
        assert!(parse_fetchable_url("  http://example.com  ").is_ok());

        for raw in [
            "file:///C:/Windows/win.ini",
            "ftp://example.com/x",
            "gopher://example.com:70/x",
            "data:text/html,<b>x</b>",
            "javascript:alert(1)",
            "example.com",
        ] {
            assert!(parse_fetchable_url(raw).is_err(), "`{raw}` must be refused");
        }
    }

    /// `https://docs.rs@127.0.0.1/` looks like docs.rs but is not.
    #[test]
    fn credentials_that_disguise_the_real_host_are_refused() {
        assert!(parse_fetchable_url("https://docs.rs@127.0.0.1/").is_err());
        assert!(parse_fetchable_url("https://user:pw@example.com/").is_err());
    }

    #[test]
    fn loopback_and_private_ranges_are_not_public() {
        for raw in [
            "127.0.0.1", // loopback — local model servers, dev APIs
            "0.0.0.0",
            "10.1.2.3", // private
            "172.16.5.4",
            "192.168.1.1",     // home routers
            "169.254.169.254", // cloud instance metadata
            "100.64.0.1",      // carrier-grade NAT
            "198.18.0.1",
            "192.0.0.1",
            "224.0.0.1", // multicast
            "255.255.255.255",
        ] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(!ip_is_public(ip), "{raw} must not be fetchable");
        }

        for raw in ["1.1.1.1", "93.184.216.34", "8.8.8.8"] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(ip_is_public(ip), "{raw} is a public address");
        }
    }

    #[test]
    fn ipv6_local_and_v4_mapped_forms_are_not_public() {
        for raw in [
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1", // v4 loopback wearing a v6 costume
            "::ffff:10.0.0.1",
            "64:ff9b::7f00:1", // NAT64 to 127.0.0.1
            "2001:db8::1",
        ] {
            let ip: IpAddr = raw.parse().unwrap();
            assert!(!ip_is_public(ip), "{raw} must not be fetchable");
        }

        let public: IpAddr = "2606:4700:4700::1111".parse().unwrap();
        assert!(ip_is_public(public));
    }

    #[tokio::test]
    async fn a_url_pointing_at_localhost_is_refused_before_any_request() {
        for raw in [
            "http://127.0.0.1:11434/api/tags",
            "http://localhost:8080/",
            "http://[::1]/",
        ] {
            let url = parse_fetchable_url(raw).expect("scheme is fine");
            let err = resolve_public_addr(&url)
                .await
                .expect_err("must not be contacted");
            assert!(
                err.to_string().contains("non-public"),
                "unexpected error for {raw}: {err}"
            );
        }
    }

    #[test]
    fn binary_content_types_are_refused() {
        for mime in [
            "application/octet-stream",
            "image/png",
            "video/mp4",
            "application/zip",
        ] {
            assert!(
                check_content_type(Some(mime)).is_err(),
                "`{mime}` is not a readable page"
            );
        }
        for mime in [
            "text/html; charset=utf-8",
            "TEXT/PLAIN",
            "application/xhtml+xml",
            "application/json",
        ] {
            assert!(check_content_type(Some(mime)).is_ok(), "`{mime}` is text");
        }
        assert!(
            check_content_type(None).is_ok(),
            "an undeclared type is left to the extractor"
        );
    }
}
