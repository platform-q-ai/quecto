// Logging utilities: API key redaction for tracing output.

/// Candidate byte values that can start a secret prefix: `s` (sk-), `g` (gsk_/gsk-),
/// or an ASCII digit (Telegram token).
///
/// All entries are ASCII (≤0x7F) so they will never appear as UTF-8 continuation
/// bytes (0x80–0xBF), guaranteeing that a position match always lands on a
/// codepoint boundary.
///
/// NOTE: Must stay in sync with the prefix checks inside `detect_secret()`. If a
/// new secret type is added (e.g. `xoxb-` for Slack, starting with `x`), add its
/// first byte here too.
const CANDIDATE_STARTS: &[u8] = b"sg0123456789";

/// Redact API keys from a string.
///
/// Matches patterns like `sk-...`, `sk-ant-...`, and similar API key prefixes.
/// Replaces the key with a redacted placeholder preserving the prefix.
///
/// Uses a skip-ahead scan (#306): instead of walking every character, the inner
/// loop advances directly to the next byte that *could* start a secret, copying
/// the clean run in one `push_str` rather than char-by-char.
pub fn redact_api_keys(input: &str) -> String {
    redact_api_keys_cow(input).into_owned()
}

/// Like [`redact_api_keys`] but returns `Cow::Borrowed` when nothing needs
/// redacting, so the common (no-secret) log line is forwarded without an
/// allocation.
pub fn redact_api_keys_cow(input: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: skip entirely if none of the triggering prefixes are present.
    if !input.contains("sk-")
        && !input.contains("gsk_")
        && !input.contains("gsk-")
        && !contains_telegram_candidate(input)
    {
        return std::borrow::Cow::Borrowed(input);
    }

    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        // i must always point at a codepoint boundary (guaranteed by initialisation
        // to 0 and subsequent advances via ch.len_utf8() or char-counted redacted_len).
        debug_assert!(
            input.is_char_boundary(i),
            "i={i} is not a codepoint boundary"
        );

        // Skip ahead to the next byte that could begin a secret.
        let next_candidate = bytes[i..]
            .iter()
            .position(|b| CANDIDATE_STARTS.contains(b))
            .map(|pos| i + pos)
            .unwrap_or(bytes.len());

        // Copy the clean run before the candidate in one shot.
        if next_candidate > i {
            result.push_str(&input[i..next_candidate]);
            i = next_candidate;
        }
        if i >= bytes.len() {
            break;
        }

        // Try to detect a secret at this position.
        if let Some((redacted_len, replacement)) = detect_secret(&input[i..]) {
            let key = &input[i..i + redacted_len];
            if replacement.is_empty() {
                result.push_str("***");
            } else {
                let prefix = extract_prefix(key, replacement);
                result.push_str(prefix);
                result.push_str("***");
            }
            i += redacted_len;
        } else {
            // Not a secret — copy the byte and advance.
            let ch = input[i..].chars().next().unwrap_or_default();
            result.push(ch);
            i += ch.len_utf8();
        }
    }

    std::borrow::Cow::Owned(result)
}

fn detect_secret(s: &str) -> Option<(usize, &'static str)> {
    detect_prefixed_key(s, &["sk-"], 8)
        .map(|len| (len, "sk-"))
        .or_else(|| detect_prefixed_key(s, &["gsk_"], 12).map(|len| (len, "gsk_")))
        .or_else(|| detect_prefixed_key(s, &["gsk-"], 12).map(|len| (len, "gsk-")))
        .or_else(|| {
            if s.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                detect_telegram_token(s).map(|len| (len, ""))
            } else {
                None
            }
        })
}

fn contains_telegram_candidate(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes
        .windows(2)
        .any(|w| w[0].is_ascii_digit() && w[1] == b':')
}

/// Detect an API key starting at the given position, return its length.
fn detect_prefixed_key(s: &str, prefixes: &[&str], min_len: usize) -> Option<usize> {
    let prefix_len = prefixes
        .iter()
        .find(|&&p| s.starts_with(p))
        .map(|p| p.len())?;
    let key_len = s[prefix_len..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .count();
    let total = prefix_len + key_len;
    if total >= min_len { Some(total) } else { None }
}

fn detect_telegram_token(s: &str) -> Option<usize> {
    let mut chars = s.chars().peekable();
    let mut digit_count = 0;

    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            digit_count += 1;
            chars.next();
        } else {
            break;
        }
    }

    if digit_count < 5 || chars.next() != Some(':') {
        return None;
    }

    let mut token_len = digit_count + 1;
    let mut suffix_count = 0;
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            token_len += 1;
            suffix_count += 1;
            chars.next();
        } else {
            break;
        }
    }

    if suffix_count >= 20 {
        Some(token_len)
    } else {
        None
    }
}

/// Extract the prefix portion of a key (e.g. "sk-" from "sk-test-123").
fn extract_prefix<'a>(key: &'a str, default_prefix: &'a str) -> &'a str {
    // For "sk-ant-..." return "sk-ant-", for "sk-..." return "sk-"
    if key.starts_with("sk-ant-") {
        "sk-ant-"
    } else if key.starts_with("gsk_") {
        "gsk_"
    } else if key.starts_with("gsk-") {
        "gsk-"
    } else {
        default_prefix
    }
}

use std::io::Write;

/// A writer that redacts API keys from each chunk before forwarding to `inner`.
struct RedactingWriter<W: Write> {
    inner: W,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // `from_utf8_lossy` already returns `Borrowed` for valid UTF-8 (the
        // common case), and `redact_api_keys_cow` returns `Borrowed` when the
        // line carries no secret — so a clean ASCII log line is forwarded with
        // no intermediate allocation.
        let lossy = String::from_utf8_lossy(buf);
        let redacted = redact_api_keys_cow(&lossy);
        self.inner.write_all(redacted.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// `MakeWriter` that produces redacting writers over stderr.
#[derive(Clone, Default)]
struct RedactingMakeWriter;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter<std::io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: std::io::stderr(),
        }
    }
}

/// Install a global tracing subscriber whose output is scrubbed of API keys.
///
/// The filter defaults to `OFF`, so this is a genuine no-op unless `RUST_LOG`
/// is set — no behaviour change by default. `try_init` never panics if a
/// subscriber is already installed. Call only from headless entrypoints
/// (the agent/UDS path); never from the REPL/TUI, where stray stderr output
/// corrupts the terminal UI.
pub fn install_redacting_subscriber() {
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::OFF.into())
        .from_env_lossy();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(RedactingMakeWriter)
        .try_init();
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
