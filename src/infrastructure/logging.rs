// Logging utilities: API key redaction for tracing output.

/// Redact API keys from a string.
///
/// Matches patterns like `sk-...`, `sk-ant-...`, and similar API key prefixes.
/// Replaces the key with a redacted placeholder preserving the prefix.
pub fn redact_api_keys(input: &str) -> String {
    // Fast path: skip scanning if no API key prefix present
    if !input.contains("sk-")
        && !input.contains("gsk_")
        && !input.contains("gsk-")
        && !contains_telegram_candidate(input)
    {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if let Some((redacted_len, replacement)) = detect_secret(&input[i..]) {
            let key = &input[i..i + redacted_len];
            if replacement.is_empty() {
                result.push_str("***");
            } else {
                let prefix = extract_prefix(key, replacement);
                result.push_str(&format!("{}***", prefix));
            }
            i += redacted_len;
        } else {
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

/// Detect an API key starting at the given position, return its byte length.
fn detect_api_key(s: &str) -> Option<usize> {
    // Match sk- followed by at least 8 alphanumeric/dash/underscore chars
    if !s.starts_with("sk-") {
        return None;
    }
    let byte_len = s
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-' || **b == b'_')
        .count();
    if byte_len >= 8 { Some(byte_len) } else { None }
}

fn detect_groq_key(s: &str) -> Option<usize> {
    if !s.starts_with("gsk_") && !s.starts_with("gsk-") {
        return None;
    }
    let byte_len = s
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-' || **b == b'_')
        .count();
    if byte_len >= 12 { Some(byte_len) } else { None }
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
}
