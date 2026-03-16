// Web fetch tool: fetch a URL and return its content as text.
//
// Strips HTML tags to produce readable text, saving LLM tokens.
// The `raw` parameter bypasses stripping for JSON APIs, markdown, etc.
//
// HTML stripping is done with simple string scanning (no regex crate)
// to avoid adding a runtime dependency.

use std::borrow::Cow;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Maximum raw download size before text extraction (5 MB).
const MAX_RAW_BYTES: usize = 5 * 1024 * 1024;

/// Default request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Fetch a URL and return its content as readable text.
#[derive(Debug)]
pub struct WebFetchTool {
    client: reqwest::Client,
    max_response_kb: u32,
    /// Allowlisted host:port pairs that bypass SSRF checks (for tests).
    #[cfg(any(test, feature = "test-support"))]
    allowed_hosts: Vec<String>,
}

impl WebFetchTool {
    /// Create with a shared `reqwest::Client` and output size cap.
    pub fn with_client(client: reqwest::Client, max_response_kb: u32) -> Self {
        Self {
            client,
            max_response_kb,
            #[cfg(any(test, feature = "test-support"))]
            allowed_hosts: Vec::new(),
        }
    }

    /// Create with default settings (for unit tests only).
    #[cfg(test)]
    fn new() -> Self {
        Self::with_client(reqwest::Client::new(), 32)
    }

    /// Create a tool that allowlists a specific host:port for SSRF bypass
    /// (for wiremock BDD tests on localhost). Other restricted hosts are
    /// still blocked, so SSRF protection scenarios continue to work.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_allowed_host(max_response_kb: u32, host_port: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            max_response_kb,
            allowed_hosts: vec![host_port.to_string()],
        }
    }

    /// Create with localhost SSRF bypass for all loopback ports (for unit tests).
    #[cfg(test)]
    fn new_allow_localhost(max_response_kb: u32) -> Self {
        Self {
            client: reqwest::Client::new(),
            max_response_kb,
            allowed_hosts: vec!["*".to_string()], // wildcard = skip all SSRF checks
        }
    }
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
        let args = arguments.to_string();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&args)
                .map_err(|e| DomainError::Tool(format!("invalid JSON: {e}")))?;

            let url = parsed
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DomainError::Tool("missing required field: url".into()))?;

            let raw = parsed.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);

            // Parse and validate URL
            let parsed_url = reqwest::Url::parse(url)
                .map_err(|e| DomainError::Tool(format!("Invalid URL: {e}")))?;

            if parsed_url.scheme() != "http" && parsed_url.scheme() != "https" {
                return Ok(ToolResult {
                    content: format!(
                        "Invalid URL scheme: only http:// and https:// are allowed. Got: {url}"
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // SSRF protection: reject internal/loopback/metadata hosts
            #[cfg(any(test, feature = "test-support"))]
            let skip_ssrf = {
                let host_port = parsed_url
                    .host_str()
                    .map(|h| {
                        if let Some(port) = parsed_url.port() {
                            format!("{}:{}", h, port)
                        } else {
                            h.to_string()
                        }
                    })
                    .unwrap_or_default();
                self.allowed_hosts
                    .iter()
                    .any(|h| h == "*" || h == &host_port)
            };
            #[cfg(not(any(test, feature = "test-support")))]
            let skip_ssrf = false;

            if !skip_ssrf {
                if let Some(host) = parsed_url.host_str() {
                    if is_restricted_host_or_ip(host) {
                        return Ok(ToolResult {
                            content: format!(
                                "Blocked: URL points to a restricted address ({host})"
                            ),
                            is_error: true,
                            image_blocks: vec![],
                        });
                    }
                }
            }

            // Fetch with timeout
            let resp = self
                .client
                .get(parsed_url)
                .timeout(REQUEST_TIMEOUT)
                .header("User-Agent", concat!("quecto/", env!("CARGO_PKG_VERSION")))
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        DomainError::Tool(format!(
                            "Request timed out after {REQUEST_TIMEOUT:?}: {url}"
                        ))
                    } else {
                        DomainError::Tool(format!("Fetch failed: {e}"))
                    }
                })?;

            if !resp.status().is_success() {
                return Ok(ToolResult {
                    content: format!("HTTP {} fetching {url}", resp.status()),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // Read body with streaming size cap
            let bytes = read_body_capped(resp, MAX_RAW_BYTES).await?;
            let body = String::from_utf8_lossy(&bytes);

            // Extract or return raw
            let content = if raw {
                body.into_owned()
            } else {
                strip_html(&body)
            };

            // Truncate to max_response_kb
            let max_bytes = self.max_response_kb as usize * 1024;
            let content = if content.len() > max_bytes {
                let mut truncated = truncate_utf8(&content, max_bytes).to_string();
                truncated.push_str(&format!(
                    "\n\n[Truncated: output exceeded {}KB limit]",
                    self.max_response_kb
                ));
                truncated
            } else {
                content
            };

            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
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

/// Check if a host string (IP or domain) is restricted.
///
/// Blocks loopback, link-local, private RFC-1918, cloud metadata IPs,
/// and known restricted domain names to prevent SSRF attacks.
///
/// `host` comes from `url::Url::host_str()` which wraps IPv6 in brackets
/// (e.g. `[::1]`), so we strip them before parsing.
fn is_restricted_host_or_ip(host: &str) -> bool {
    // Strip IPv6 brackets: host_str() returns "[::1]" for IPv6
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_restricted_ip(ip);
    }
    // Known restricted domain names
    matches!(
        bare,
        "localhost" | "metadata.google.internal" | "metadata.google.internal."
    )
}

/// Check if an IP address is in a restricted range.
fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()       // 127.0.0.0/8
            || v4.is_private()     // 10/8, 172.16/12, 192.168/16
            || v4.is_link_local()  // 169.254.0.0/16 (AWS IMDS)
            || v4.is_unspecified() // 0.0.0.0
            || v4.is_broadcast() // 255.255.255.255
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()       // ::1
            || v6.is_unspecified() // ::
        }
    }
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
///
/// Note: `remove_tag_blocks` is called 6 times, each doing a
/// `to_ascii_lowercase()` of the remaining text. For the 5 MB cap this is
/// ~60 MB of transient allocations. Acceptable for a cold-path tool call;
/// a single-pass state machine would be more efficient if profiling shows
/// this matters.
pub fn strip_html(html: &str) -> String {
    let stripped = remove_tag_blocks(html, "script");
    let stripped = remove_tag_blocks(&stripped, "style");
    let stripped = remove_tag_blocks(&stripped, "nav");
    let stripped = remove_tag_blocks(&stripped, "footer");
    let stripped = remove_tag_blocks(&stripped, "header");
    let stripped = remove_tag_blocks(&stripped, "noscript");

    let text = tags_to_text(&stripped);
    let text = decode_entities(&text);
    collapse_whitespace(&text)
}

/// Remove all occurrences of `<tag ...>...</tag>` (case-insensitive).
///
/// Note: nested same-name tags (e.g. `<nav><nav>inner</nav>leak</nav>`)
/// will leave content after the first closing tag. This is acceptable for
/// readability stripping (not security sanitisation).
fn remove_tag_blocks(html: &str, tag: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let mut pos = 0;
    while pos < html.len() {
        if let Some(start) = lower[pos..].find(&open) {
            let abs_start = pos + start;
            let after_tag = abs_start + open.len();
            if after_tag < lower.len() {
                let next = lower.as_bytes()[after_tag];
                if next == b' ' || next == b'>' || next == b'/' || next == b'\t' || next == b'\n' {
                    result.push_str(&html[pos..abs_start]);
                    if let Some(end) = lower[abs_start..].find(&close) {
                        pos = abs_start + end + close.len();
                        continue;
                    }
                    return result;
                }
            }
            result.push_str(&html[pos..after_tag]);
            pos = after_tag;
        } else {
            result.push_str(&html[pos..]);
            break;
        }
    }
    result
}

/// Convert HTML tags to text: block tags become newlines, others are stripped.
///
/// Uses `eq_ignore_ascii_case` per tag to avoid allocating a lowercase copy
/// for every tag in the document.
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
    let bytes = html.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(end_offset) = html[i..].find('>') {
                let tag_content = &html[i + 1..i + end_offset];
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
                i += end_offset + 1;
            } else {
                result.push('<');
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Decode common HTML entities.
fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = text[i..].find(';') {
                let entity = &text[i + 1..i + semi];
                if let Some(decoded) = decode_entity(entity) {
                    result.push(decoded);
                    i += semi + 1;
                    continue;
                }
            }
            result.push('&');
            i += 1;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
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

/// Append `line` to `out` with runs of whitespace collapsed to single spaces,
/// leading/trailing whitespace trimmed. No per-line allocation.
fn push_collapsed_line(out: &mut String, line: &str) {
    let mut prev_space = true; // true = trim leading spaces
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    // Trim trailing space
    if out.ends_with(' ') {
        out.pop();
    }
}

/// Collapse runs of whitespace into single spaces, blank lines into single
/// blank lines, and trim each line. Writes directly into a single output
/// buffer to avoid per-line heap allocations.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(256 * 1024));
    let mut consecutive_blank = 0_u32;
    let mut first_line = true;

    for line in text.lines() {
        if line.chars().all(|c| c.is_whitespace()) {
            consecutive_blank += 1;
            if consecutive_blank <= 1 {
                if !first_line {
                    out.push('\n');
                }
                first_line = false;
            }
            continue;
        }

        consecutive_blank = 0;
        if !first_line {
            out.push('\n');
        }
        first_line = false;
        push_collapsed_line(&mut out, line);
    }

    // Trim leading/trailing blank lines in-place
    while out.ends_with('\n') {
        out.pop();
    }
    if let Some(start) = out.find(|c: char| c != '\n') {
        if start > 0 {
            out.drain(..start);
        }
    }
    out
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
