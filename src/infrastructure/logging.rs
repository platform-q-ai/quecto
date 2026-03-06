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
    // Fast path: skip entirely if none of the triggering prefixes are present.
    if !input.contains("sk-")
        && !input.contains("gsk_")
        && !input.contains("gsk-")
        && !contains_telegram_candidate(input)
    {
        return input.to_string();
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

    result
}

fn detect_secret(s: &str) -> Option<(usize, &'static str)> {
    detect_api_key(s)
        .map(|len| (len, "sk-"))
        .or_else(|| detect_groq_key(s).map(|len| (len, "gsk_")))
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
fn detect_api_key(s: &str) -> Option<usize> {
    // Match sk- followed by at least 8 alphanumeric/dash/underscore chars
    if !s.starts_with("sk-") {
        return None;
    }
    let key_len = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .count();
    if key_len >= 8 { Some(key_len) } else { None }
}

fn detect_groq_key(s: &str) -> Option<usize> {
    if !s.starts_with("gsk_") && !s.starts_with("gsk-") {
        return None;
    }
    let key_len = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .count();
    if key_len >= 12 { Some(key_len) } else { None }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_openai_key() {
        let input = "Provider configured with key sk-secret-key-12345";
        let result = redact_api_keys(input);
        assert!(!result.contains("sk-secret-key-12345"));
        assert!(result.contains("sk-***"));
    }

    #[test]
    fn test_redact_anthropic_key() {
        let input = "Using key sk-ant-abcdefghijk123456789";
        let result = redact_api_keys(input);
        assert!(!result.contains("sk-ant-abcdefghijk123456789"));
        assert!(result.contains("sk-ant-***"));
    }

    #[test]
    fn test_no_redaction_without_key() {
        let input = "This is a normal log message";
        let result = redact_api_keys(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_redact_multiple_keys() {
        let input = "key1=sk-aaaa-bbbb-cccc key2=sk-dddd-eeee-ffff";
        let result = redact_api_keys(input);
        assert!(!result.contains("aaaa-bbbb-cccc"));
        assert!(!result.contains("dddd-eeee-ffff"));
        assert_eq!(result.matches("sk-***").count(), 2);
    }

    #[test]
    fn test_short_prefix_not_redacted() {
        let input = "sk-ab is too short";
        let result = redact_api_keys(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_redact_preserves_surrounding() {
        let input = "before sk-test-key-abcdef after";
        let result = redact_api_keys(input);
        assert_eq!(result, "before sk-*** after");
    }

    #[test]
    fn test_redact_groq_style_key() {
        let input = "groq key gsk_abcdefghijklmnopqrstuvwxyz";
        let result = redact_api_keys(input);
        assert!(!result.contains("gsk_abcdefghijklmnopqrstuvwxyz"));
        assert!(result.contains("gsk_***"));
    }

    #[test]
    fn test_redact_telegram_bot_token() {
        let input = "telegram token 123456789:AAAbbbCCCDDDeeeFFF111222333";
        let result = redact_api_keys(input);
        assert!(!result.contains("123456789:AAAbbbCCCDDDeeeFFF111222333"));
        assert!(result.contains("telegram token ***"));
    }

    #[test]
    fn test_detect_api_key_valid() {
        assert!(detect_api_key("sk-test-key-12345").is_some());
    }

    #[test]
    fn test_detect_api_key_too_short() {
        assert!(detect_api_key("sk-ab").is_none());
    }

    #[test]
    fn test_detect_api_key_not_sk() {
        assert!(detect_api_key("pk-test-key-12345").is_none());
    }

    // --- #306: fast-path correctness after prefix-scan optimisation ---

    #[test]
    fn fast_path_returns_same_string_no_alloc_for_clean_lines() {
        // Lines without any key prefix must take the fast path and return identical content.
        let input = "HTTP 200 OK — response received in 42ms";
        let result = redact_api_keys(input);
        assert_eq!(result, input);
    }

    #[test]
    fn partial_prefix_sk_dash_not_matching_minimum_length_is_left_intact() {
        // "sk-" appearing in a word like "disk-" should not be redacted
        let input = "reading from /dev/disk-0 via udev";
        let result = redact_api_keys(input);
        assert_eq!(result, input);
    }

    #[test]
    fn redact_mid_line_key_leaves_surrounding_text() {
        let input = "auth=sk-abcdefghijklmno next=foo";
        let result = redact_api_keys(input);
        assert!(!result.contains("abcdefghijklmno"));
        assert!(result.starts_with("auth=sk-***"));
        assert!(result.ends_with("next=foo"));
    }
}
