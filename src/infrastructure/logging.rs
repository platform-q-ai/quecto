// Logging utilities: API key redaction for tracing output.

/// Redact API keys from a string.
///
/// Matches patterns like `sk-...`, `sk-ant-...`, and similar API key prefixes.
/// Replaces the key with a redacted placeholder preserving the prefix.
pub fn redact_api_keys(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some((i, _)) = chars.peek().copied() {
        if let Some(redacted_len) = detect_api_key(&input[i..]) {
            let key = &input[i..i + redacted_len];
            let prefix = extract_prefix(key);
            result.push_str(&format!("{}***", prefix));
            // Skip past the key
            for _ in 0..redacted_len {
                chars.next();
            }
        } else {
            result.push(input.as_bytes()[i] as char);
            chars.next();
        }
    }

    result
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

/// Extract the prefix portion of a key (e.g. "sk-" from "sk-test-123").
fn extract_prefix(key: &str) -> &str {
    // For "sk-ant-..." return "sk-ant-", for "sk-..." return "sk-"
    if key.starts_with("sk-ant-") {
        "sk-ant-"
    } else {
        "sk-"
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
