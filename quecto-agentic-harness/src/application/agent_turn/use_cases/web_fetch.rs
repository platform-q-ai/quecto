//! Application-owned web fetch policy, orchestration, and content transformation.
use crate::domain::network_destination::{authorize_destination, is_fetchable_name};
use std::{future::Future, net::IpAddr, pin::Pin, sync::Arc};

#[path = "web_fetch_main_content.rs"]
pub mod main_content;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedHttpUrl(url::Url);
impl ParsedHttpUrl {
    pub fn parse(value: &str) -> Result<Self, url::ParseError> {
        url::Url::parse(value).map(Self)
    }

    pub fn as_url(&self) -> &url::Url {
        &self.0
    }
}
/// What the port is asked to fetch. The use case builds one only from an
/// `Allowed` gate, but the type does not enforce that: the adapter
/// authorizes every URL and redirect hop itself (#1942 ledger row 13).
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub url: ParsedHttpUrl,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpStatus {
    pub code: u16,
    pub reason: Option<String>,
}
impl HttpStatus {
    pub fn new(code: u16, reason: Option<String>) -> Self {
        Self { code, reason }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchOutcome {
    /// A body and the `Content-Type` it was served with, when there was one.
    SuccessBody {
        body: Vec<u8>,
        content_type: Option<String>,
    },
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
    /// A destination the fetch would have reached is not a permitted one:
    /// an address a name resolved to, or a redirect hop (#1942). Nothing was
    /// sent to it.
    Refused(String),
    /// A redirect that cannot be followed safely: not exactly one `Location`,
    /// or one that is empty or not a URL (#1942). Nothing was sent to any
    /// target.
    BadRedirect(String),
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
    /// Content that is not text: named, not shown (#2165).
    Binary {
        content_type: Option<String>,
        bytes: usize,
    },
    UnsupportedScheme,
    RestrictedInitialHost(String),
    NonSuccessStatus(HttpStatus),
}
/// How an HTML body is returned (#2165).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlView {
    /// Readable text of the page's main content when it marks one, else of
    /// the whole page.
    MainContent,
    /// Readable text of the whole page.
    WholePage,
    /// The markup as served.
    Markup,
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
    pub async fn execute(
        &self,
        url: &str,
        view: HtmlView,
    ) -> Result<WebFetchResult, WebFetchError> {
        let parsed = url::Url::parse(url).map_err(|e| WebFetchError::InvalidUrl(e.to_string()))?;
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
            FetchOutcome::SuccessBody { body, content_type } => {
                // By what was served (#2165): HTML is returned in the view
                // asked for, other text comes back as it is, anything else
                // is named rather than decoded into noise.
                let content = match (kind_of(content_type.as_deref(), &body), view) {
                    (BodyKind::Binary, _) => {
                        return Ok(WebFetchResult::Binary {
                            content_type,
                            bytes: body.len(),
                        });
                    }
                    (BodyKind::Html, HtmlView::MainContent) => {
                        main_content::readable_html(decoded(&body).as_ref())
                    }
                    (BodyKind::Html, HtmlView::WholePage) => strip_html(decoded(&body).as_ref()),
                    (BodyKind::Html, HtmlView::Markup) | (BodyKind::Text, _) => {
                        decoded(&body).into_owned()
                    }
                };
                Ok(WebFetchResult::Success(truncate_output(
                    content,
                    self.max_response_kb,
                )))
            }
        }
    }
}
/// What a fetched body is, by its served type or, without one, its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyKind {
    Html,
    Text,
    Binary,
}

/// Media types returned as text (the served type's essence, lowercased).
fn is_text_media(essence: &str) -> bool {
    essence.starts_with("text/")
        || essence.ends_with("+json")
        || essence.ends_with("+xml")
        || matches!(
            essence,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/ecmascript"
                | "application/x-javascript"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/x-sh"
                | "application/sql"
                | "application/graphql"
                | "application/x-ndjson"
                | "application/jsonl"
                | "application/jsonlines"
                | "application/csv"
        )
}

/// A served type that says nothing about the content: generic, unknown,
/// empty or not of the `type/subtype` form.
fn says_nothing(essence: &str) -> bool {
    let well_formed = essence
        .split_once('/')
        .is_some_and(|(kind, sub)| !kind.is_empty() && !sub.is_empty());
    matches!(
        essence,
        "application/octet-stream" | "binary/octet-stream" | "application/unknown"
    ) || !well_formed
}

fn kind_of(content_type: Option<&str>, body: &[u8]) -> BodyKind {
    let essence = content_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    match essence.as_deref() {
        Some("text/html" | "application/xhtml+xml") => BodyKind::Html,
        Some(essence) if is_text_media(essence) => BodyKind::Text,
        // A generic or unknown type says nothing about the bytes: text
        // served as octet-stream is common (S3, CDNs; #2177 review).
        Some(essence) if says_nothing(essence) => sniff(body),
        Some(_) => BodyKind::Binary,
        None => sniff(body),
    }
}

/// Untyped content: text when it is valid UTF-8 with no NUL, HTML when it
/// also opens like a document.
fn sniff(body: &[u8]) -> BodyKind {
    let Ok(text) = std::str::from_utf8(body) else {
        return BodyKind::Binary;
    };
    if text.contains('\0') {
        return BodyKind::Binary;
    }
    let opening = document_opening(text).as_bytes();
    let opens_with = |prefix: &[u8]| {
        opening
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    if opens_with(b"<!doctype html") || opens_with(b"<html") {
        BodyKind::Html
    } else {
        BodyKind::Text
    }
}

/// A text body, without a leading byte-order mark.
fn decoded(body: &[u8]) -> std::borrow::Cow<'_, str> {
    match String::from_utf8_lossy(body) {
        std::borrow::Cow::Borrowed(text) => {
            std::borrow::Cow::Borrowed(text.trim_start_matches('\u{feff}'))
        }
        std::borrow::Cow::Owned(text) => {
            std::borrow::Cow::Owned(text.trim_start_matches('\u{feff}').to_owned())
        }
    }
}

/// Where a document's own markup starts: past a byte-order mark, an XML
/// prolog, comments and whitespace (#2177 review).
fn document_opening(text: &str) -> &str {
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        let trimmed = rest.trim_start();
        let skipped = [("<?", "?>"), ("<!--", "-->")]
            .iter()
            .find(|(open, _)| trimmed.starts_with(open))
            .and_then(|(_, close)| trimmed.find(close).map(|end| &trimmed[end + close.len()..]));
        match skipped {
            Some(after) => rest = after,
            None => return trimmed,
        }
    }
}

fn classify(url: url::Url) -> ExecutionGate {
    if matches!(url.scheme(), "http" | "https") {
        match restriction(&url) {
            Some(reason) => ExecutionGate::RestrictedInitialHost(reason),
            None => ExecutionGate::Allowed(ParsedHttpUrl(url)),
        }
    } else {
        ExecutionGate::UnsupportedScheme
    }
}
/// Why the URL's own host may not be fetched, if it may not. An address
/// literal must be public (#1942): that allowlist is the security boundary,
/// applied again by the adapter to every address a name resolves to and to
/// every redirect hop. A local name is refused early, as a courtesy
/// ([`is_fetchable_name`]); the adapter applies it to every hop too.
fn restriction(url: &url::Url) -> Option<String> {
    let address = match url.host() {
        Some(url::Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(url::Host::Ipv6(v6)) => IpAddr::V6(v6),
        Some(url::Host::Domain(name)) => {
            return match is_fetchable_name(name) {
                true => None,
                false => Some(format!("{name} is a local name")),
            };
        }
        None => return Some("the URL names no host".to_owned()),
    };
    authorize_destination(address)
        .err()
        .map(|refused| refused.to_string())
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
    text_of_markup(&remove_configured_tag_blocks(html))
}

/// Steps 2-5 of [`strip_html`], for markup whose stripped blocks are gone.
fn text_of_markup(stripped: &str) -> String {
    let text = tags_to_text(stripped);
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
    remove_tag_blocks(html, STRIPPED_BLOCK_TAGS)
}

/// Remove all occurrences of `<tag ...>...</tag>` blocks for the given tags,
/// as [`remove_configured_tag_blocks`] does for its own.
fn remove_tag_blocks(html: &str, stripped: &[&'static str]) -> String {
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;
    // Tags with no close anywhere after some point have none after any later
    // point either: searched once, so many unclosed tags stay linear (#2177
    // review).
    let mut never_closed: Vec<&'static str> = Vec::new();

    while pos < html.len() {
        let Some(tag_start_rel) = html[pos..].find('<') else {
            result.push_str(&html[pos..]);
            break;
        };
        let tag_start = pos + tag_start_rel;
        let Some(tag) = open_tag_of(stripped, html, tag_start) else {
            result.push_str(&html[pos..=tag_start]);
            pos = tag_start + 1;
            continue;
        };

        result.push_str(&html[pos..tag_start]);
        let close = match never_closed.contains(&tag) {
            true => None,
            false => find_configured_close_tag(html, tag_start + 1, tag),
        };
        if let Some(close_start) = close {
            let close_end = html[close_start..]
                .find('>')
                .map(|idx| close_start + idx + 1)
                .unwrap_or(html.len());
            pos = close_end;
        } else {
            // Never closed: drop only the opening tag, never the rest of
            // the page (#2165).
            never_closed.push(tag);
            pos = html[tag_start..]
                .find('>')
                .map_or(html.len(), |idx| tag_start + idx + 1);
        }
    }

    result
}

fn open_tag_of(stripped: &[&'static str], html: &str, tag_start: usize) -> Option<&'static str> {
    stripped
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

/// `</tag>` at `tag_start`, with optional whitespace before its `>`
/// (#2165 review). The whitespace run is read once per `</tag`, so the
/// callers' scans stay linear.
fn specific_close_tag_at(html: &str, tag_start: usize, tag: &str) -> bool {
    let after_slash = tag_start + 2;
    let after_name = after_slash + tag.len();
    let closes = || {
        html.get(after_name..).is_some_and(|rest| {
            rest.trim_start_matches(|c: char| c.is_ascii_whitespace())
                .starts_with('>')
        })
    };
    html.as_bytes().get(tag_start..after_slash) == Some(b"</")
        && html
            .get(after_slash..after_name)
            .is_some_and(|name| name.eq_ignore_ascii_case(tag))
        && closes()
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

        // Past quoted attribute values (#2165 review); a stray `<` is text.
        if let TagEnd::At(end_offset) = tag_end(&html.as_bytes()[abs_open..]) {
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
            // A stray or unended '<' is a literal character. The next '<'
            // is where `tag_end` stopped, so every byte is read at most
            // twice.
            result.push('<');
            pos = abs_open + 1;
        }
    }

    result.push_str(&html[pos..]);
    result
}

/// Where a tag opened by the `<` at `rest[0]` ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagEnd {
    /// Its `>`.
    At(usize),
    /// Another `<` came first, outside quotes: the first was stray.
    Stray(usize),
    /// Neither follows: no whole tag does.
    Never,
}

/// Find a tag's `>` in one forward pass, past quoted attribute values (a
/// quote opens a value only after `=`; #2165 review).
///
/// A quote that never closes is no value: the tag then ends at the first
/// `>`, or the next starts at the first `<`, after its start, whatever the
/// quotes. Both lie past that quote, and that quote character appears
/// nowhere after it, so no later tag reads to the end for it again: at
/// most two such reads a page, one per quote character, so linear.
fn tag_end(rest: &[u8]) -> TagEnd {
    let mut quote = None;
    let mut after_equals = false;
    let mut first_bracket = None;
    for (at, &byte) in rest.iter().enumerate().skip(1) {
        if first_bracket.is_none() {
            first_bracket = match byte {
                b'>' => Some(TagEnd::At(at)),
                b'<' => Some(TagEnd::Stray(at)),
                _ => None,
            };
        }
        if let Some(open) = quote {
            if byte == open {
                quote = None;
            }
            continue;
        }
        match byte {
            b'>' => return TagEnd::At(at),
            b'<' => return TagEnd::Stray(at),
            b'"' | b'\'' if after_equals => {
                quote = Some(byte);
                after_equals = false;
                continue;
            }
            _ => {}
        }
        after_equals = byte == b'=' || (after_equals && byte.is_ascii_whitespace());
    }
    // Brackets inside values that closed were values; only an unclosed
    // quote falls back to the first bracket.
    match quote {
        Some(_) => first_bracket.unwrap_or(TagEnd::Never),
        None => TagEnd::Never,
    }
}

/// Decode common HTML entities. Operates on `&str` so multibyte characters
/// are preserved instead of being re-interpreted as Latin-1 bytes.
/// Longest entity text looked for, `&` to `;` (`&thetasym;` is 10).
const MAX_ENTITY_BYTES: usize = 12;

fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(amp) = text[pos..].find('&') {
        let abs_amp = pos + amp;
        result.push_str(&text[pos..abs_amp]);

        // An entity is short: look for its `;` only within reach, so many
        // `&` with a far `;` stay linear (#2177 review).
        let reach = text.len().min(abs_amp + MAX_ENTITY_BYTES);
        let window = text.get(abs_amp..reach).unwrap_or("");
        if let Some(semi) = window.find(';') {
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
#[path = "web_fetch_content_tests.rs"]
mod content_tests;
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
