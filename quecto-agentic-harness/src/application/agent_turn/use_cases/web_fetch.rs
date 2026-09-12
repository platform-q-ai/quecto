//! Capability-local destination authorization for `web_fetch`.
//!
//! This policy belongs in the application layer because it is a rule of one
//! use case, not an enterprise-wide invariant. URL parsing, name resolution,
//! redirects, sockets, and HTTP remain infrastructure mechanisms; they supply
//! owned, backend-neutral facts here and proceed only after [`Allowed`](
//! WebDestinationAuthorization::Allowed).

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::Arc;

/// Application input after the interface has decoded the public tool request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFetchRequest {
    pub url: String,
    pub raw: bool,
}

/// Neutral request passed to the outbound content-fetching mechanism.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchWebContentRequest {
    pub url: String,
}

/// Mechanism result before application content policy is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedWebContent {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Mechanical failures reported by the outbound adapter without vendor types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchWebContentError {
    InvalidUrl(String),
    InvalidScheme,
    DestinationDenied(String),
    RedirectLimitExceeded,
    TimedOut,
    ResponseTooLarge {
        actual_bytes: Option<usize>,
        max_bytes: usize,
    },
    Resolution(String),
    Connection(String),
    Read(String),
    Transport(String),
}

/// Object-safe outward effect owned by the application capability.
pub trait FetchWebContent: Send + Sync {
    fn fetch(
        &self,
        request: FetchWebContentRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchedWebContent, FetchWebContentError>> + Send + '_>>;
}

/// Application result ready for the interface's `ToolResult` translation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebFetchResult {
    pub content: String,
    pub is_error: bool,
}

/// Failures which remain errors rather than ordinary error-valued tool output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebFetchError {
    InvalidUrl(String),
    RedirectLimitExceeded,
    TimedOut,
    ResponseTooLarge {
        actual_bytes: Option<usize>,
        max_bytes: usize,
    },
    Resolution(String),
    Connection(String),
    Read(String),
    Transport(String),
}

/// Coordinates exactly one fetch effect, then applies capability-local content policy.
pub struct WebFetchUseCase {
    content: Arc<dyn FetchWebContent>,
    max_response_kb: u32,
}

impl WebFetchUseCase {
    pub fn new(content: Arc<dyn FetchWebContent>, max_response_kb: u32) -> Self {
        Self {
            content,
            max_response_kb,
        }
    }

    pub async fn execute(&self, request: WebFetchRequest) -> Result<WebFetchResult, WebFetchError> {
        let requested_url = request.url;
        let fetched = match self
            .content
            .fetch(FetchWebContentRequest {
                url: requested_url.clone(),
            })
            .await
        {
            Ok(fetched) => fetched,
            Err(FetchWebContentError::InvalidScheme) => {
                return Ok(WebFetchResult {
                    content: format!(
                        "Invalid URL scheme: only http:// and https:// are allowed. Got: {requested_url}"
                    ),
                    is_error: true,
                });
            }
            Err(FetchWebContentError::DestinationDenied(host)) => {
                return Ok(WebFetchResult {
                    content: format!("Blocked: URL points to a restricted address ({host})"),
                    is_error: true,
                });
            }
            Err(error) => return Err(map_fetch_error(error)),
        };

        if !(200..300).contains(&fetched.status) {
            return Ok(WebFetchResult {
                content: format!("HTTP {} fetching {requested_url}", fetched.status),
                is_error: true,
            });
        }

        let body = String::from_utf8_lossy(&fetched.body);
        let content = if request.raw {
            body.into_owned()
        } else {
            strip_html(&body)
        };
        let max_bytes = self.max_response_kb as usize * 1024;
        let content = truncate_output(content, max_bytes, self.max_response_kb);

        Ok(WebFetchResult {
            content,
            is_error: false,
        })
    }
}

fn map_fetch_error(error: FetchWebContentError) -> WebFetchError {
    match error {
        FetchWebContentError::InvalidUrl(message) => WebFetchError::InvalidUrl(message),
        FetchWebContentError::RedirectLimitExceeded => WebFetchError::RedirectLimitExceeded,
        FetchWebContentError::TimedOut => WebFetchError::TimedOut,
        FetchWebContentError::ResponseTooLarge {
            actual_bytes,
            max_bytes,
        } => WebFetchError::ResponseTooLarge {
            actual_bytes,
            max_bytes,
        },
        FetchWebContentError::Resolution(message) => WebFetchError::Resolution(message),
        FetchWebContentError::Connection(message) => WebFetchError::Connection(message),
        FetchWebContentError::Read(message) => WebFetchError::Read(message),
        FetchWebContentError::Transport(message) => WebFetchError::Transport(message),
        FetchWebContentError::InvalidScheme | FetchWebContentError::DestinationDenied(_) => {
            unreachable!("tool-output errors are handled before mechanical error mapping")
        }
    }
}

fn truncate_output(content: String, max_bytes: usize, max_response_kb: u32) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    let mut truncated = truncate_utf8(&content, max_bytes).to_owned();
    truncated.push_str(&format!(
        "\n\n[Truncated: output exceeded {max_response_kb}KB limit]"
    ));
    truncated
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Strip HTML into the established readable-text representation.
pub fn strip_html(html: &str) -> String {
    let stripped = remove_configured_tag_blocks(html);
    let text = tags_to_text(&stripped);
    let text = decode_entities(&text);
    collapse_whitespace(&text)
}

const STRIPPED_BLOCK_TAGS: &[&str] = &["script", "style", "nav", "footer", "header", "noscript"];

fn remove_configured_tag_blocks(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut position = 0;
    while position < html.len() {
        let Some(relative_start) = html[position..].find('<') else {
            result.push_str(&html[position..]);
            break;
        };
        let tag_start = position + relative_start;
        let Some(tag) = configured_open_tag_at(html, tag_start) else {
            result.push_str(&html[position..=tag_start]);
            position = tag_start + 1;
            continue;
        };
        result.push_str(&html[position..tag_start]);
        if let Some(close_start) = find_configured_close_tag(html, tag_start + 1, tag) {
            position = html[close_start..]
                .find('>')
                .map(|index| close_start + index + 1)
                .unwrap_or(html.len());
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

fn find_configured_close_tag(html: &str, mut position: usize, tag: &str) -> Option<usize> {
    while let Some(relative_start) = html[position..].find('<') {
        let tag_start = position + relative_start;
        if specific_close_tag_at(html, tag_start, tag) {
            return Some(tag_start);
        }
        position = tag_start + 1;
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
    let mut position = 0;
    while let Some(relative_open) = html[position..].find('<') {
        let open = position + relative_open;
        result.push_str(&html[position..open]);
        if let Some(end_offset) = html[open..].find('>') {
            let tag_content = &html[open + 1..open + end_offset];
            let trimmed = tag_content.trim().trim_start_matches('/');
            let tag_end = trimmed
                .find(|character: char| character.is_whitespace() || character == '/')
                .unwrap_or(trimmed.len());
            let tag_name = &trimmed[..tag_end];
            if tag_name.eq_ignore_ascii_case("br")
                || BLOCK_TAGS
                    .iter()
                    .any(|tag| tag_name.eq_ignore_ascii_case(tag))
            {
                result.push('\n');
            }
            position = open + end_offset + 1;
        } else {
            result.push('<');
            position = open + 1;
        }
    }
    result.push_str(&html[position..]);
    result
}

fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(relative_ampersand) = text[position..].find('&') {
        let ampersand = position + relative_ampersand;
        result.push_str(&text[position..ampersand]);
        if let Some(semicolon) = text[ampersand..].find(';') {
            let entity = &text[ampersand + 1..ampersand + semicolon];
            if let Some(decoded) = decode_entity(entity) {
                result.push(decoded);
                position = ampersand + semicolon + 1;
                continue;
            }
        }
        result.push('&');
        position = ampersand + 1;
    }
    result.push_str(&text[position..]);
    result
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ if entity.starts_with('#') => {
            let number = &entity[1..];
            if let Some(hexadecimal) = number
                .strip_prefix('x')
                .or_else(|| number.strip_prefix('X'))
            {
                u32::from_str_radix(hexadecimal, 16)
                    .ok()
                    .and_then(char::from_u32)
            } else {
                number.parse::<u32>().ok().and_then(char::from_u32)
            }
        }
        _ => None,
    }
}

fn push_collapsed_line(output: &mut String, line: &str) {
    let mut previous_space = true;
    for character in line.chars() {
        if character.is_whitespace() {
            if !previous_space {
                output.push(' ');
                previous_space = true;
            }
        } else {
            output.push(character);
            previous_space = false;
        }
    }
    if output.ends_with(' ') {
        output.pop();
    }
}

fn collapse_whitespace(text: &str) -> String {
    let mut output = String::with_capacity(text.len().min(256 * 1024));
    let mut consecutive_blank = 0_u32;
    let mut first_line = true;
    for line in text.lines() {
        if line.chars().all(char::is_whitespace) {
            consecutive_blank += 1;
            if consecutive_blank <= 1 {
                if !first_line {
                    output.push('\n');
                }
                first_line = false;
            }
            continue;
        }
        consecutive_blank = 0;
        if !first_line {
            output.push('\n');
        }
        first_line = false;
        push_collapsed_line(&mut output, line);
    }
    while output.ends_with('\n') {
        output.pop();
    }
    if let Some(start) = output.find(|character: char| character != '\n') {
        if start > 0 {
            output.drain(..start);
        }
    }
    output
}

/// Backend-neutral facts about one URL or one resolved connection candidate.
///
/// `effective_port` is optional deliberately: incomplete normalization must be
/// representable so that the policy can fail closed rather than invent a
/// default outside the URL mechanism. `candidate` is populated when
/// authorizing an address returned by resolution or eligible for connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebDestinationTarget {
    scheme: String,
    host: String,
    effective_port: Option<u16>,
    candidate: Option<IpAddr>,
}

impl WebDestinationTarget {
    /// Own and normalize the backend-neutral target facts.
    pub fn new(
        scheme: impl Into<String>,
        host: impl Into<String>,
        effective_port: impl Into<Option<u16>>,
        candidate: Option<IpAddr>,
    ) -> Self {
        let scheme = scheme.into().to_ascii_lowercase();
        let host = normalize_host(host.into());
        Self {
            scheme,
            host,
            effective_port: effective_port.into(),
            candidate,
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn effective_port(&self) -> Option<u16> {
        self.effective_port
    }

    pub fn candidate(&self) -> Option<IpAddr> {
        self.candidate
    }

    fn has_complete_normalized_identity(&self) -> bool {
        matches!(self.effective_port, Some(1..=u16::MAX))
            && self.scheme == self.scheme.to_ascii_lowercase()
            && self.host == normalize_host(self.host.clone())
            && !self.host.is_empty()
    }
}

/// The only decisions on which `web_fetch` execution may proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "network execution requires an explicit Allowed decision"]
pub enum WebDestinationAuthorization {
    Allowed,
    Denied,
}

/// Sole application authority for `web_fetch` schemes and destinations.
#[derive(Clone, Debug, Default)]
pub struct WebDestinationPolicy {
    #[cfg(any(test, feature = "test-support"))]
    test_destination: Option<TestDestination>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct TestDestination {
    host: String,
    port: u16,
}

impl WebDestinationPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Permit exactly one normalized host/port destination in test builds.
    ///
    /// This is intentionally not a wildcard. It exists only so deterministic
    /// tests can use a local server while all other restricted destinations
    /// remain denied. Scheme authorization is never bypassed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_destination(host: impl Into<String>, port: u16) -> Self {
        Self {
            test_destination: Some(TestDestination {
                host: normalize_host(host.into()),
                port,
            }),
        }
    }

    /// Authorize a normalized URL target or resolved connection candidate.
    ///
    /// Authorization is conjunctive and affirmative: every required fact must
    /// be classified, and a supplied address candidate must itself be public
    /// (or belong to the exact test-only destination).
    pub fn authorize(&self, target: &WebDestinationTarget) -> WebDestinationAuthorization {
        let test_destination_allowed = self.test_destination_allowed(target);
        let identity_allowed = target.has_complete_normalized_identity()
            && is_allowed_scheme(&target.scheme)
            && (is_public_host(&target.host) || test_destination_allowed);
        let candidate_allowed = target
            .candidate
            .is_none_or(|candidate| is_public_ip(candidate) || test_destination_allowed);

        if identity_allowed && candidate_allowed {
            WebDestinationAuthorization::Allowed
        } else {
            WebDestinationAuthorization::Denied
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn test_destination_allowed(&self, target: &WebDestinationTarget) -> bool {
        self.test_destination.as_ref().is_some_and(|allowed| {
            allowed.port > 0
                && target.host == allowed.host
                && target.effective_port == Some(allowed.port)
                && is_well_formed_host(&allowed.host)
        })
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn test_destination_allowed(&self, _target: &WebDestinationTarget) -> bool {
        false
    }
}

fn normalize_host(host: String) -> String {
    let lowercase = host.to_ascii_lowercase();
    lowercase
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .filter(|inner| inner.parse::<Ipv6Addr>().is_ok())
        .unwrap_or(&lowercase)
        .to_owned()
}

fn is_allowed_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https")
}

fn is_public_host(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(is_public_ip)
        .unwrap_or_else(|_| is_public_dns_name(host))
}

#[cfg(any(test, feature = "test-support"))]
fn is_well_formed_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok() || is_well_formed_dns_name(host)
}

fn is_public_dns_name(host: &str) -> bool {
    is_well_formed_dns_name(host)
        && !matches!(
            host,
            "localhost" | "localhost." | "metadata.google.internal" | "metadata.google.internal."
        )
}

fn is_well_formed_dns_name(host: &str) -> bool {
    let without_root = host.strip_suffix('.').unwrap_or(host);
    let labels: Vec<_> = without_root.split('.').collect();
    let has_public_shape = without_root.len() <= 253
        && labels.len() >= 2
        && !without_root
            .bytes()
            .all(|byte| byte == b'.' || byte.is_ascii_digit());

    has_public_shape
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    // Public unicast is the complement of the IANA special-purpose blocks.
    // Keep this classification here, alongside the scheme rule, so adapters
    // cannot quietly acquire a second destination authority.
    !(ip.is_unspecified()
        || ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || ipv4_in_cidr(ip, Ipv4Addr::new(0, 0, 0, 0), 8)
        || ipv4_in_cidr(ip, Ipv4Addr::new(100, 64, 0, 0), 10)
        || ipv4_in_cidr(ip, Ipv4Addr::new(192, 0, 0, 0), 24)
        || ipv4_in_cidr(ip, Ipv4Addr::new(192, 88, 99, 0), 24)
        || ipv4_in_cidr(ip, Ipv4Addr::new(198, 18, 0, 0), 15)
        || ipv4_in_cidr(ip, Ipv4Addr::new(240, 0, 0, 0), 4))
}

fn ipv4_in_cidr(ip: Ipv4Addr, network: Ipv4Addr, prefix: u32) -> bool {
    assert!(prefix <= 32, "IPv4 prefix invariant");
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(network) & mask
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    // Globally routable unicast currently occupies 2000::/3. Explicitly
    // remove documentation and special assignment blocks from that positive
    // classification; all other IPv6 classes fail closed.
    ipv6_in_cidr(ip, Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 0), 32)
        && !ipv6_in_cidr(ip, Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20)
}

fn ipv6_in_cidr(ip: Ipv6Addr, network: Ipv6Addr, prefix: u32) -> bool {
    assert!(prefix <= 128, "IPv6 prefix invariant");
    let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
    u128::from(ip) & mask == u128::from(network) & mask
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
