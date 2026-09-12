// Web fetch tool: fetch a URL and return its content as text.
//
// Destination authorization is owned by the application policy. This adapter
// owns only URL parsing, DNS, redirect, connection, HTTP, and body mechanisms.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use crate::application::agent_turn::use_cases::web_fetch::{
    WebDestinationAuthorization, WebDestinationPolicy, WebDestinationTarget,
};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Maximum raw download size before text extraction (5 MB).
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;
/// Default request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Reqwest's historic default is ten followed redirects.
const MAX_REDIRECTS: usize = 10;

type ResolutionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, DomainError>> + Send + 'a>>;
pub type ClientBuilderFactory = Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync>;

/// Fetch-controlled DNS seam. Implementations return candidates once; the
/// adapter authorizes that exact set and pins it into the connector.
trait DestinationResolver: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolutionFuture<'a>;
}

#[derive(Debug, Default)]
struct SystemDestinationResolver;

impl DestinationResolver for SystemDestinationResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolutionFuture<'a> {
        Box::pin(async move {
            tokio::net::lookup_host((host, port))
                .await
                .map(|addresses| addresses.collect())
                .map_err(|error| {
                    DomainError::Tool(format!("Fetch failed: DNS resolution failed: {error}"))
                })
        })
    }
}

/// Fetch a URL and return its content as readable text.
pub struct WebFetchTool {
    max_response_kb: u32,
    request_timeout: Duration,
    destination_policy: WebDestinationPolicy,
    resolver: Arc<dyn DestinationResolver>,
    client_builder_factory: ClientBuilderFactory,
}

impl std::fmt::Debug for WebFetchTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebFetchTool")
            .field("max_response_kb", &self.max_response_kb)
            .field("request_timeout", &self.request_timeout)
            .field("destination_policy", &self.destination_policy)
            .finish_non_exhaustive()
    }
}

impl WebFetchTool {
    /// Create with the output cap and the default replayable HTTP/TLS recipe.
    /// A dedicated client is built for each authorized hop so automatic
    /// redirects and a second DNS lookup can never bypass the destination
    /// decision.
    pub fn new(max_response_kb: u32) -> Self {
        Self::with_client_builder_factory(
            Arc::new(crate::infrastructure::providers::default_client_builder),
            max_response_kb,
        )
    }

    /// Create from a replayable configured-client recipe.
    ///
    /// Reqwest clients are opaque after construction: an injected, already-built
    /// `Client` cannot be augmented with a per-hop DNS pin or have proxy and
    /// redirect policy safely overridden. Accept the recipe instead, then apply
    /// those mandatory security settings after every authorized resolution.
    /// This preserves TLS trust, identity, timeout, header, and pool settings
    /// intentionally configured by the caller rather than replacing them with
    /// reqwest defaults.
    pub fn with_client_builder_factory(
        client_builder_factory: ClientBuilderFactory,
        max_response_kb: u32,
    ) -> Self {
        Self {
            max_response_kb,
            request_timeout: REQUEST_TIMEOUT,
            destination_policy: WebDestinationPolicy::new(),
            resolver: Arc::new(SystemDestinationResolver),
            client_builder_factory,
        }
    }

    /// Permit one exact local destination for deterministic tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_allowed_host(max_response_kb: u32, host_port: &str) -> Self {
        let (host, port) = parse_test_destination(host_port)
            .expect("test destination must be one exact host:port pair");
        let candidate = host
            .parse()
            .expect("test destination must use one exact IP literal");
        Self {
            max_response_kb,
            request_timeout: REQUEST_TIMEOUT,
            destination_policy: WebDestinationPolicy::with_test_destination(host, port, candidate),
            resolver: Arc::new(SystemDestinationResolver),
            client_builder_factory: Arc::new(reqwest::Client::builder),
        }
    }

    #[cfg(test)]
    fn with_test_dependencies(
        max_response_kb: u32,
        destination_policy: WebDestinationPolicy,
        resolver: Arc<dyn DestinationResolver>,
    ) -> Self {
        Self::with_test_dependencies_and_timeout(
            max_response_kb,
            REQUEST_TIMEOUT,
            destination_policy,
            resolver,
        )
    }

    #[cfg(test)]
    fn with_test_dependencies_and_timeout(
        max_response_kb: u32,
        request_timeout: Duration,
        destination_policy: WebDestinationPolicy,
        resolver: Arc<dyn DestinationResolver>,
    ) -> Self {
        assert!(
            !request_timeout.is_zero(),
            "request timeout must be positive"
        );
        Self::with_test_dependencies_and_client_factory(
            max_response_kb,
            request_timeout,
            destination_policy,
            resolver,
            Arc::new(reqwest::Client::builder),
        )
    }

    #[cfg(test)]
    fn with_test_dependencies_and_client_factory(
        max_response_kb: u32,
        request_timeout: Duration,
        destination_policy: WebDestinationPolicy,
        resolver: Arc<dyn DestinationResolver>,
        client_builder_factory: ClientBuilderFactory,
    ) -> Self {
        assert!(
            !request_timeout.is_zero(),
            "request timeout must be positive"
        );
        Self {
            max_response_kb,
            request_timeout,
            destination_policy,
            resolver,
            client_builder_factory,
        }
    }

    async fn fetch_hop(
        &self,
        url: &reqwest::Url,
        deadline: Instant,
    ) -> Result<reqwest::Response, FetchHopError> {
        let host = url.host_str().ok_or(FetchHopError::Denied)?;
        let port = url.port_or_known_default().ok_or(FetchHopError::Denied)?;
        let literal_candidate = normalized_ip_literal(host);
        let target = WebDestinationTarget::new(url.scheme(), host, port, literal_candidate);
        if self.destination_policy.authorize(&target) != WebDestinationAuthorization::Allowed {
            return Err(FetchHopError::Denied);
        }

        let authorized_addresses = authorized_candidates(
            &self.destination_policy,
            url,
            if let Some(ip) = literal_candidate {
                vec![SocketAddr::new(ip, port)]
            } else {
                self.resolver
                    .resolve(host, port)
                    .await
                    .map_err(FetchHopError::Mechanical)?
            },
        );
        if authorized_addresses.is_empty() {
            return Err(FetchHopError::Denied);
        }
        let every_connector_candidate_is_authorized = authorized_addresses.iter().all(|address| {
            let target = WebDestinationTarget::new(url.scheme(), host, port, Some(address.ip()));
            self.destination_policy.authorize(&target) == WebDestinationAuthorization::Allowed
        });
        assert!(
            every_connector_candidate_is_authorized,
            "every connector candidate must have explicit authorization"
        );

        // No proxy, no automatic redirect, and no resolver fallback. The only
        // connector candidates are the exact addresses authorized above.
        let client = (self.client_builder_factory)()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &authorized_addresses)
            .build()
            .map_err(|error| {
                FetchHopError::Mechanical(DomainError::Tool(format!("Fetch failed: {error}")))
            })?;
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(FetchHopError::TimedOut);
        };
        client
            .get(url.clone())
            .timeout(remaining)
            .header("User-Agent", concat!("quecto/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    FetchHopError::TimedOut
                } else {
                    FetchHopError::Mechanical(DomainError::Tool(format!("Fetch failed: {error}")))
                }
            })
    }
}

#[derive(Debug)]
enum FetchHopError {
    Denied,
    TimedOut,
    Mechanical(DomainError),
}

fn authorized_candidates(
    policy: &WebDestinationPolicy,
    url: &reqwest::Url,
    candidates: Vec<SocketAddr>,
) -> Vec<SocketAddr> {
    let Some(host) = url.host_str() else {
        return Vec::new();
    };
    let Some(port) = url.port_or_known_default() else {
        return Vec::new();
    };
    candidates
        .into_iter()
        .map(|candidate| SocketAddr::new(candidate.ip(), port))
        .filter(|candidate| {
            let target = WebDestinationTarget::new(url.scheme(), host, port, Some(candidate.ip()));
            policy.authorize(&target) == WebDestinationAuthorization::Allowed
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalized_ip_literal(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
        .parse()
        .ok()
}

#[cfg(any(test, feature = "test-support"))]
fn parse_test_destination(host_port: &str) -> Option<(String, u16)> {
    if let Ok(address) = host_port.parse::<SocketAddr>() {
        return Some((address.ip().to_string(), address.port())).filter(|(_, port)| *port > 0);
    }
    let (host, port) = host_port.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    (!host.is_empty() && port > 0).then(|| (host.to_owned(), port))
}

impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch a URL and return its content as readable text. \
                          Strips HTML tags by default to save tokens. \
                          Use raw mode for JSON APIs or markdown files."
                .into(),
            parameters_schema: Cow::Borrowed(
                r#"{"type":"object","properties":{"url":{"type":"string","description":"URL to fetch (http or https)"},"raw":{"type":"boolean","description":"Return raw body without HTML stripping (default: false)"}},"required":["url"]}"#,
            ),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let arguments = arguments.to_owned();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&arguments)
                .map_err(|error| DomainError::Tool(format!("invalid JSON: {error}")))?;
            let requested_url = parsed
                .get("url")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| DomainError::Tool("missing required field: url".into()))?;
            let raw = parsed
                .get("raw")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let mut current_url = reqwest::Url::parse(requested_url)
                .map_err(|error| DomainError::Tool(format!("Invalid URL: {error}")))?;

            let deadline = Instant::now() + self.request_timeout;
            let operation = async {
                let mut redirects = 0_usize;
                let response = loop {
                    let response = match self.fetch_hop(&current_url, deadline).await {
                        Ok(response) => response,
                        Err(FetchHopError::Denied) => {
                            return Ok(denied_destination_result(&current_url, requested_url));
                        }
                        Err(FetchHopError::TimedOut) => {
                            return Err(timeout_error(requested_url, self.request_timeout));
                        }
                        Err(FetchHopError::Mechanical(error)) => return Err(error),
                    };
                    if !is_followed_redirect_status(response.status()) {
                        break response;
                    }
                    let mut locations =
                        response.headers().get_all(reqwest::header::LOCATION).iter();
                    let Some(location) = locations.next() else {
                        return Ok(denied_destination_result(&current_url, requested_url));
                    };
                    if locations.next().is_some() {
                        return Ok(denied_destination_result(&current_url, requested_url));
                    }
                    let location = match location.to_str() {
                        Ok(location) if !location.is_empty() => location,
                        _ => return Ok(denied_destination_result(&current_url, requested_url)),
                    };
                    let next_url = match current_url.join(location) {
                        Ok(next_url) => next_url,
                        Err(_) => {
                            return Ok(denied_destination_result(&current_url, requested_url));
                        }
                    };
                    redirects += 1;
                    if redirects > MAX_REDIRECTS {
                        return Err(DomainError::Tool(format!(
                            "Fetch failed: redirect limit exceeded fetching {requested_url}"
                        )));
                    }
                    // The next iteration authorizes the redirect target before its
                    // DNS lookup or request, then repeats candidate authorization.
                    current_url = next_url;
                };

                present_response(response, requested_url, raw, self.max_response_kb).await
            };

            match tokio::time::timeout_at(deadline, operation).await {
                Ok(Err(_)) if Instant::now() >= deadline => {
                    Err(timeout_error(requested_url, self.request_timeout))
                }
                Ok(result) => result,
                Err(_) => Err(timeout_error(requested_url, self.request_timeout)),
            }
        })
    }
}

fn is_followed_redirect_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::MOVED_PERMANENTLY
            | reqwest::StatusCode::FOUND
            | reqwest::StatusCode::SEE_OTHER
            | reqwest::StatusCode::TEMPORARY_REDIRECT
            | reqwest::StatusCode::PERMANENT_REDIRECT
    )
}

fn timeout_error(requested_url: &str, request_timeout: Duration) -> DomainError {
    DomainError::Tool(format!(
        "Request timed out after {request_timeout:?}: {requested_url}"
    ))
}

fn denied_destination_result(url: &reqwest::Url, requested_url: &str) -> ToolResult {
    let content = if matches!(url.scheme(), "http" | "https") {
        format!(
            "Blocked: URL points to a restricted address ({})",
            url.host_str().unwrap_or("unknown")
        )
    } else {
        format!("Invalid URL scheme: only http:// and https:// are allowed. Got: {requested_url}")
    };
    ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

async fn present_response(
    response: reqwest::Response,
    requested_url: &str,
    raw: bool,
    max_response_kb: u32,
) -> Result<ToolResult, DomainError> {
    if !response.status().is_success() {
        return Ok(ToolResult {
            content: format!("HTTP {} fetching {requested_url}", response.status()),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        });
    }
    let bytes = read_body_capped(response, MAX_RAW_BYTES).await?;
    let body = String::from_utf8_lossy(&bytes);
    let content = if raw {
        body.into_owned()
    } else {
        strip_html(&body)
    };
    let max_bytes = max_response_kb as usize * 1024;
    let content = if content.len() > max_bytes {
        let mut truncated = truncate_utf8(&content, max_bytes).to_owned();
        truncated.push_str(&format!(
            "\n\n[Truncated: output exceeded {max_response_kb}KB limit]"
        ));
        truncated
    } else {
        content
    };
    Ok(ToolResult {
        content,
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}

/// Read response body up to `max_bytes` using streaming chunks.
///
/// Aborts mid-stream if the body exceeds the cap, avoiding OOM from
/// servers that send large bodies without a Content-Length header.
async fn read_body_capped(
    mut resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, DomainError> {
    // Pre-flight: reject if Content-Length is known and too large
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(DomainError::Tool(format!(
                "Response too large: {len} bytes (max {max_bytes})"
            )));
        }
    }

    let mut buf = Vec::with_capacity(max_bytes.min(256 * 1024));
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| DomainError::Tool(format!("Failed to read response body: {e}")))?
    {
        buf.extend_from_slice(&chunk);
        if buf.len() > max_bytes {
            return Err(DomainError::Tool(format!(
                "Response too large: >{max_bytes} bytes (max {max_bytes})"
            )));
        }
    }
    Ok(buf)
}

/// Truncate a string to at most `max_bytes`, respecting UTF-8 boundaries.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Strip HTML to produce readable plain text.
///
/// Strategy:
/// 1. Remove `<script>`, `<style>`, `<nav>`, `<footer>`, `<header>`,
///    `<noscript>` blocks entirely
/// 2. Convert block-closing tags to newlines
/// 3. Strip remaining tags
/// 4. Decode common HTML entities
/// 5. Collapse whitespace
pub fn strip_html(html: &str) -> String {
    let stripped = remove_configured_tag_blocks(html);
    let text = tags_to_text(&stripped);
    let text = decode_entities(&text);
    collapse_whitespace(&text)
}

const STRIPPED_BLOCK_TAGS: &[&str] = &["script", "style", "nav", "footer", "header", "noscript"];

/// Remove all occurrences of configured `<tag ...>...</tag>` blocks (case-insensitive).
///
/// Note: nested same-name tags (e.g. `<nav><nav>inner</nav>leak</nav>`)
/// will leave content after the first closing tag. This is acceptable for
/// readability stripping (not security sanitisation).
fn remove_configured_tag_blocks(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;

    while pos < html.len() {
        let Some(tag_start_rel) = html[pos..].find('<') else {
            result.push_str(&html[pos..]);
            break;
        };
        let tag_start = pos + tag_start_rel;
        let Some(tag) = configured_open_tag_at(html, tag_start) else {
            result.push_str(&html[pos..=tag_start]);
            pos = tag_start + 1;
            continue;
        };

        result.push_str(&html[pos..tag_start]);
        if let Some(close_start) = find_configured_close_tag(html, tag_start + 1, tag) {
            let close_end = html[close_start..]
                .find('>')
                .map(|idx| close_start + idx + 1)
                .unwrap_or(html.len());
            pos = close_end;
        } else {
            break;
        }
    }

    result
}

fn configured_open_tag_at(html: &str, tag_start: usize) -> Option<&'static str> {
    STRIPPED_BLOCK_TAGS
        .iter()
        .copied()
        .find(|tag| specific_open_tag_at(html, tag_start, tag))
}

fn specific_open_tag_at(html: &str, tag_start: usize, tag: &str) -> bool {
    let after_lt = tag_start + 1;
    let after_name = after_lt + tag.len();
    html.get(after_lt..after_name)
        .is_some_and(|name| name.eq_ignore_ascii_case(tag))
        && html
            .as_bytes()
            .get(after_name)
            .is_some_and(|next| matches!(*next, b' ' | b'>' | b'/' | b'\t' | b'\n'))
}

fn find_configured_close_tag(html: &str, mut pos: usize, tag: &str) -> Option<usize> {
    while let Some(tag_start_rel) = html[pos..].find('<') {
        let tag_start = pos + tag_start_rel;
        if specific_close_tag_at(html, tag_start, tag) {
            return Some(tag_start);
        }
        pos = tag_start + 1;
    }
    None
}

fn specific_close_tag_at(html: &str, tag_start: usize, tag: &str) -> bool {
    let after_slash = tag_start + 2;
    let after_name = after_slash + tag.len();
    html.as_bytes().get(tag_start..after_slash) == Some(b"</")
        && html
            .get(after_slash..after_name)
            .is_some_and(|name| name.eq_ignore_ascii_case(tag))
        && html.as_bytes().get(after_name) == Some(&b'>')
}

/// Convert HTML tags to text: block tags become newlines, others are stripped.
///
/// Uses `eq_ignore_ascii_case` per tag to avoid allocating a lowercase copy
/// for every tag in the document. All text between tags is copied as UTF-8
/// substrings, so multibyte characters (e.g. `é`) are preserved.
fn tags_to_text(html: &str) -> String {
    const BLOCK_TAGS: &[&str] = &[
        "p",
        "div",
        "li",
        "tr",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "blockquote",
        "pre",
    ];

    let mut result = String::with_capacity(html.len());
    let mut pos = 0;

    while let Some(open) = html[pos..].find('<') {
        let abs_open = pos + open;
        result.push_str(&html[pos..abs_open]);

        if let Some(end_offset) = html[abs_open..].find('>') {
            let tag_content = &html[abs_open + 1..abs_open + end_offset];
            let trimmed = tag_content.trim().trim_start_matches('/');
            let tag_end = trimmed
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(trimmed.len());
            let tag_name = &trimmed[..tag_end];

            if tag_name.eq_ignore_ascii_case("br")
                || BLOCK_TAGS.iter().any(|t| tag_name.eq_ignore_ascii_case(t))
            {
                result.push('\n');
            }
            pos = abs_open + end_offset + 1;
        } else {
            // Unclosed '<' — treat it as a literal character and continue.
            result.push('<');
            pos = abs_open + 1;
        }
    }

    result.push_str(&html[pos..]);
    result
}

/// Decode common HTML entities. Operates on `&str` so multibyte characters
/// are preserved instead of being re-interpreted as Latin-1 bytes.
fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(amp) = text[pos..].find('&') {
        let abs_amp = pos + amp;
        result.push_str(&text[pos..abs_amp]);

        if let Some(semi) = text[abs_amp..].find(';') {
            let entity = &text[abs_amp + 1..abs_amp + semi];
            if let Some(decoded) = decode_entity(entity) {
                result.push(decoded);
                pos = abs_amp + semi + 1;
                continue;
            }
        }
        result.push('&');
        pos = abs_amp + 1;
    }

    result.push_str(&text[pos..]);
    result
}

/// Decode a single HTML entity (without & and ;).
fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ if entity.starts_with('#') => {
            let num_str = &entity[1..];
            if let Some(hex) = num_str
                .strip_prefix('x')
                .or_else(|| num_str.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else {
                num_str.parse::<u32>().ok().and_then(char::from_u32)
            }
        }
        _ => None,
    }
}

#[path = "web_fetch_text.rs"]
mod text;

use text::collapse_whitespace;

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "web_fetch_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "web_fetch_ssrf_tests.rs"]
mod ssrf_tests;
