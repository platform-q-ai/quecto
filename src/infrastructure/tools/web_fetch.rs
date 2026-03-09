// Web fetch tool: fetch a URL and return its content as text.
//
// Strips HTML tags to produce readable text, saving LLM tokens.
// The `raw` parameter bypasses stripping for JSON APIs, markdown, etc.
//
// HTML stripping is done with simple string scanning (no regex crate)
// to avoid adding a runtime dependency.

use std::borrow::Cow;
use std::future::Future;
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
}

impl WebFetchTool {
    /// Create with a shared `reqwest::Client` and output size cap.
    pub fn with_client(client: reqwest::Client, max_response_kb: u32) -> Self {
        Self {
            client,
            max_response_kb,
        }
    }

    /// Create with default settings (for tests).
    #[cfg(test)]
    fn new() -> Self {
        Self::with_client(reqwest::Client::new(), 32)
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

            // Scheme validation
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Ok(ToolResult {
                    content: format!(
                        "Invalid URL scheme: only http:// and https:// are allowed. Got: {url}"
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // Fetch with timeout
            let resp = self
                .client
                .get(url)
                .timeout(REQUEST_TIMEOUT)
                .header("User-Agent", "quecto/0.19.0")
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

            // Read body with size cap
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

/// Read response body up to `max_bytes`, returning an error if exceeded.
async fn read_body_capped(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, DomainError> {
    let content_length = resp.content_length().unwrap_or(0) as usize;
    if content_length > max_bytes {
        return Err(DomainError::Tool(format!(
            "Response too large: {content_length} bytes (max {max_bytes})"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| DomainError::Tool(format!("Failed to read response body: {e}")))?;
    if bytes.len() > max_bytes {
        return Err(DomainError::Tool(format!(
            "Response too large: {} bytes (max {max_bytes})",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
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
    // 1. Remove block elements that add noise
    let stripped = remove_tag_blocks(html, "script");
    let stripped = remove_tag_blocks(&stripped, "style");
    let stripped = remove_tag_blocks(&stripped, "nav");
    let stripped = remove_tag_blocks(&stripped, "footer");
    let stripped = remove_tag_blocks(&stripped, "header");
    let stripped = remove_tag_blocks(&stripped, "noscript");

    // 2+3. Walk the remaining HTML, converting tags to text
    let text = tags_to_text(&stripped);

    // 4. Decode HTML entities
    let text = decode_entities(&text);

    // 5. Collapse whitespace
    collapse_whitespace(&text)
}

/// Remove all occurrences of `<tag ...>...</tag>` (case-insensitive).
fn remove_tag_blocks(html: &str, tag: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let mut pos = 0;
    while pos < html.len() {
        if let Some(start) = lower[pos..].find(&open) {
            let abs_start = pos + start;
            // Make sure it's actually a tag (followed by space, > or /)
            let after_tag = abs_start + open.len();
            if after_tag < lower.len() {
                let next = lower.as_bytes()[after_tag];
                if next == b' ' || next == b'>' || next == b'/' || next == b'\t' || next == b'\n' {
                    result.push_str(&html[pos..abs_start]);
                    // Find closing tag
                    if let Some(end) = lower[abs_start..].find(&close) {
                        pos = abs_start + end + close.len();
                        continue;
                    }
                    // No closing tag — skip to end
                    return result;
                }
            }
            // Not a real tag match, include up to and past it
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
                let tag_lower = tag_content.trim().to_ascii_lowercase();
                // Strip leading / to normalise closing tags
                let tag_name = tag_lower
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("");

                if tag_name == "br" || BLOCK_TAGS.contains(&tag_name) {
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
            // Not a valid entity, pass through
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

/// Collapse runs of whitespace into single spaces, blank lines into single
/// blank lines, and trim each line.
fn collapse_whitespace(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut consecutive_blank = 0_u32;

    for line in text.lines() {
        let trimmed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            consecutive_blank += 1;
            if consecutive_blank <= 1 {
                lines.push(String::new());
            }
        } else {
            consecutive_blank = 0;
            lines.push(trimmed);
        }
    }

    // Trim leading/trailing blank lines
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_definition() {
        let tool = WebFetchTool::new();
        let def = tool.definition();
        assert_eq!(def.name.as_ref(), "web_fetch");
        assert!(def.description.contains("Fetch"));
    }

    #[test]
    fn test_strip_html_basic() {
        let html = "<p>Hello <b>world</b></p>";
        let text = strip_html(html);
        assert!(text.contains("Hello world"), "got: {text}");
    }

    #[test]
    fn test_strip_html_removes_script() {
        let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
        let text = strip_html(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_strip_html_removes_style() {
        let html = "<style>.foo { color: red; }</style><p>Content</p>";
        let text = strip_html(html);
        assert!(!text.contains("color"));
        assert!(text.contains("Content"));
    }

    #[test]
    fn test_strip_html_removes_nav_footer_header() {
        let html = "<nav>Menu</nav><main>Content</main><footer>Copyright</footer>";
        let text = strip_html(html);
        assert!(!text.contains("Menu"));
        assert!(text.contains("Content"));
        assert!(!text.contains("Copyright"));
    }

    #[test]
    fn test_strip_html_block_newlines() {
        let html = "<p>First</p><p>Second</p>";
        let text = strip_html(html);
        assert!(text.contains("First\n"), "got: {text:?}");
        assert!(text.contains("Second"));
    }

    #[test]
    fn test_strip_html_br() {
        let html = "Line 1<br>Line 2<br/>Line 3";
        let text = strip_html(html);
        assert!(text.contains("Line 1\n"), "got: {text:?}");
        assert!(text.contains("Line 2\n"), "got: {text:?}");
    }

    #[test]
    fn test_decode_entity_named() {
        assert_eq!(decode_entity("amp"), Some('&'));
        assert_eq!(decode_entity("lt"), Some('<'));
        assert_eq!(decode_entity("gt"), Some('>'));
        assert_eq!(decode_entity("quot"), Some('"'));
        assert_eq!(decode_entity("apos"), Some('\''));
        assert_eq!(decode_entity("nbsp"), Some(' '));
    }

    #[test]
    fn test_decode_entity_numeric() {
        assert_eq!(decode_entity("#65"), Some('A'));
        assert_eq!(decode_entity("#x41"), Some('A'));
        assert_eq!(decode_entity("#x2603"), Some('☃'));
    }

    #[test]
    fn test_decode_entities_in_text() {
        assert_eq!(decode_entities("&amp; &lt; &gt;"), "& < >");
        assert_eq!(decode_entities("hello&nbsp;world"), "hello world");
        assert_eq!(decode_entities("&#65;"), "A");
    }

    #[test]
    fn test_truncate_utf8_ascii() {
        assert_eq!(truncate_utf8("hello world", 5), "hello");
    }

    #[test]
    fn test_truncate_utf8_boundary() {
        let s = "café"; // é is 2 bytes
        assert_eq!(truncate_utf8(s, 4), "caf");
        assert_eq!(truncate_utf8(s, 5), "café");
    }

    #[test]
    fn test_truncate_utf8_no_truncation() {
        assert_eq!(truncate_utf8("short", 100), "short");
    }

    #[test]
    fn test_strip_html_collapses_whitespace() {
        let html = "<p>  lots   of    spaces  </p>";
        let text = strip_html(html);
        assert_eq!(text, "lots of spaces");
    }

    #[test]
    fn test_strip_html_multiline_collapse() {
        let html = "<p>A</p>\n\n\n\n\n<p>B</p>";
        let text = strip_html(html);
        assert!(!text.contains("\n\n\n"), "got: {text:?}");
    }

    #[test]
    fn test_strip_html_list_items() {
        let html = "<ul><li>One</li><li>Two</li><li>Three</li></ul>";
        let text = strip_html(html);
        assert!(text.contains("One"));
        assert!(text.contains("Two"));
        assert!(text.contains("Three"));
    }

    #[test]
    fn test_strip_html_headings() {
        let html = "<h1>Title</h1><p>Paragraph</p>";
        let text = strip_html(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Paragraph"));
    }

    #[test]
    fn test_strip_html_plain_text_passthrough() {
        let text = strip_html("Just plain text, no HTML.");
        assert_eq!(text, "Just plain text, no HTML.");
    }

    #[test]
    fn test_remove_tag_blocks_case_insensitive() {
        let html = "<SCRIPT>bad</SCRIPT>good";
        let result = remove_tag_blocks(html, "script");
        assert!(!result.contains("bad"));
        assert!(result.contains("good"));
    }

    #[test]
    fn test_remove_tag_blocks_with_attributes() {
        let html = r#"<script type="text/javascript">bad</script>good"#;
        let result = remove_tag_blocks(html, "script");
        assert!(!result.contains("bad"));
        assert!(result.contains("good"));
    }

    #[tokio::test]
    async fn test_missing_url() {
        let tool = WebFetchTool::new();
        let result = tool.execute(r#"{"wrong":"field"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let tool = WebFetchTool::new();
        let result = tool.execute("not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_scheme() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(r#"{"url":"ftp://example.com"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid URL scheme"));
    }

    #[tokio::test]
    async fn test_invalid_scheme_file() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(r#"{"url":"file:///etc/passwd"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid URL scheme"));
    }

    #[tokio::test]
    async fn test_fetch_html_strips_tags() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let html = r#"<html><head><title>Test</title><style>body{}</style></head>
            <body><nav>Menu</nav><h1>Hello</h1><p>World</p><footer>Foot</footer></body></html>"#;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(html))
            .mount(&server)
            .await;

        let tool = WebFetchTool::with_client(reqwest::Client::new(), 32);
        let result = tool
            .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
            .await
            .unwrap();
        assert!(!result.is_error, "error: {}", result.content);
        assert!(result.content.contains("Hello"), "got: {}", result.content);
        assert!(result.content.contains("World"), "got: {}", result.content);
        assert!(!result.content.contains("Menu"), "nav not stripped");
        assert!(!result.content.contains("Foot"), "footer not stripped");
        assert!(!result.content.contains("body{}"), "style not stripped");
    }

    #[tokio::test]
    async fn test_fetch_raw_mode() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let json_body = r#"{"key":"value","items":[1,2,3]}"#;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(json_body))
            .mount(&server)
            .await;

        let tool = WebFetchTool::with_client(reqwest::Client::new(), 32);
        let result = tool
            .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, json_body);
    }

    #[tokio::test]
    async fn test_fetch_http_error() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let tool = WebFetchTool::with_client(reqwest::Client::new(), 32);
        let result = tool
            .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("404"));
    }

    #[tokio::test]
    async fn test_fetch_truncates_large_response() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let big_body = "A".repeat(2048);

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&big_body))
            .mount(&server)
            .await;

        let tool = WebFetchTool::with_client(reqwest::Client::new(), 1); // 1KB cap
        let result = tool
            .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.len() < 2048);
        assert!(result.content.contains("[Truncated"));
    }

    #[tokio::test]
    async fn test_fetch_accepts_shared_client() {
        let client = reqwest::Client::new();
        let tool = WebFetchTool::with_client(client, 32);
        assert_eq!(tool.definition().name.as_ref(), "web_fetch");
    }
}
