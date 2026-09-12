//! Application-owned web fetch policy, orchestration, and content transformation.
use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedHttpUrl(reqwest::Url);
impl ParsedHttpUrl {
    pub fn as_url(&self) -> &reqwest::Url {
        &self.0
    }
}
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub url: ParsedHttpUrl,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpStatus(pub u16);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    SuccessBody(Vec<u8>),
    NonSuccessStatus(HttpStatus),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchFailure {
    TimedOut,
    TooLarge {
        actual_bytes: Option<usize>,
        max_bytes: usize,
    },
    Read(String),
    Transport(String),
}
pub trait FetchWebContent: Send + Sync {
    fn fetch<'a>(
        &'a self,
        request: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebFetchError {
    InvalidUrl(String),
    Fetch(FetchFailure),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebFetchResult {
    Success(String),
    UnsupportedScheme,
    RestrictedInitialHost(String),
    NonSuccessStatus(HttpStatus),
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum ExecutionGate {
    Allowed(ParsedHttpUrl),
    UnsupportedScheme,
    RestrictedInitialHost(String),
}

pub struct WebFetchUseCase {
    fetcher: Arc<dyn FetchWebContent>,
    max_response_kb: u32,
}
impl WebFetchUseCase {
    pub fn new(fetcher: Arc<dyn FetchWebContent>, max_response_kb: u32) -> Self {
        Self {
            fetcher,
            max_response_kb,
        }
    }
    pub async fn execute(&self, url: &str, raw: bool) -> Result<WebFetchResult, WebFetchError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|e| WebFetchError::InvalidUrl(e.to_string()))?;
        let request = match classify(parsed) {
            ExecutionGate::Allowed(url) => FetchRequest { url },
            ExecutionGate::UnsupportedScheme => return Ok(WebFetchResult::UnsupportedScheme),
            ExecutionGate::RestrictedInitialHost(host) => {
                return Ok(WebFetchResult::RestrictedInitialHost(host));
            }
        };
        match self
            .fetcher
            .fetch(&request)
            .await
            .map_err(WebFetchError::Fetch)?
        {
            FetchOutcome::NonSuccessStatus(status) => Ok(WebFetchResult::NonSuccessStatus(status)),
            FetchOutcome::SuccessBody(bytes) => {
                let body = String::from_utf8_lossy(&bytes);
                let content = if raw {
                    body.into_owned()
                } else {
                    strip_html(&body)
                };
                Ok(WebFetchResult::Success(truncate_output(
                    content,
                    self.max_response_kb,
                )))
            }
        }
    }
}
fn classify(url: reqwest::Url) -> ExecutionGate {
    if matches!(url.scheme(), "http" | "https") {
        let host = url.host_str().unwrap_or_default();
        if restricted_host(host) {
            ExecutionGate::RestrictedInitialHost(host.to_owned())
        } else {
            ExecutionGate::Allowed(ParsedHttpUrl(url))
        }
    } else {
        ExecutionGate::UnsupportedScheme
    }
}
fn restricted_host(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(host);
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        }
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => matches!(
            bare,
            "localhost" | "metadata.google.internal" | "metadata.google.internal."
        ),
    }
}
fn truncate_output(content: String, kb: u32) -> String {
    let max = kb as usize * 1024;
    if content.len() <= max {
        return content;
    }
    let mut out = truncate_utf8(&content, max).to_owned();
    out.push_str(&format!("\n\n[Truncated: output exceeded {kb}KB limit]"));
    out
}
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Fake {
        calls: AtomicUsize,
        outcome: FetchOutcome,
    }
    impl FetchWebContent for Fake {
        fn fetch<'a>(
            &'a self,
            _: &'a FetchRequest,
        ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let out = self.outcome.clone();
            Box::pin(async move { Ok(out) })
        }
    }
    fn use_case(outcome: FetchOutcome) -> (Arc<Fake>, WebFetchUseCase) {
        let f = Arc::new(Fake {
            calls: AtomicUsize::new(0),
            outcome,
        });
        (f.clone(), WebFetchUseCase::new(f, 32))
    }
    #[tokio::test]
    async fn rejection_categories_never_call_port() {
        for (url, expected) in [
            ("not a url", "error"),
            ("ftp://example.com", "scheme"),
            ("http://localhost", "host"),
            ("http://10.0.0.1", "host"),
            ("http://[::1]", "host"),
        ] {
            let (f, u) = use_case(FetchOutcome::SuccessBody(vec![]));
            let got = u.execute(url, false).await;
            match expected {
                "error" => assert!(got.is_err()),
                "scheme" => assert_eq!(got.unwrap(), WebFetchResult::UnsupportedScheme),
                "host" => assert!(matches!(
                    got.unwrap(),
                    WebFetchResult::RestrictedInitialHost(_)
                )),
                _ => unreachable!(),
            }
            assert_eq!(f.calls.load(Ordering::SeqCst), 0, "{url}");
        }
    }
    #[tokio::test]
    async fn accepted_baseline_hosts_call_port_once() {
        for url in [
            "https://example.com",
            "http://8.8.8.8",
            "http://[2001:db8::1]",
        ] {
            let (f, u) = use_case(FetchOutcome::SuccessBody(b"ok".to_vec()));
            assert_eq!(
                u.execute(url, true).await.unwrap(),
                WebFetchResult::Success("ok".into())
            );
            assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        }
    }
    #[tokio::test]
    async fn non_success_is_preserved_without_body_shape() {
        let (_, u) = use_case(FetchOutcome::NonSuccessStatus(HttpStatus(404)));
        assert_eq!(
            u.execute("https://example.com", false).await.unwrap(),
            WebFetchResult::NonSuccessStatus(HttpStatus(404))
        );
    }
    #[test]
    fn readable_and_utf8_helpers_remain_owned_here() {
        assert_eq!(strip_html("<p>Hello&nbsp;world</p>"), "Hello world");
        assert!(truncate_output("éé".into(), 0).starts_with("\n\n[Truncated"));
    }
}
