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
    assert!(detect_prefixed_key("sk-test-key-12345", &["sk-"], 8).is_some());
}

#[test]
fn test_detect_api_key_too_short() {
    assert!(detect_prefixed_key("sk-ab", &["sk-"], 8).is_none());
}

#[test]
fn test_detect_api_key_not_sk() {
    assert!(detect_prefixed_key("pk-test-key-12345", &["sk-"], 8).is_none());
}

#[test]
fn test_detect_prefixed_key_branches() {
    // valid with exact min length
    assert_eq!(detect_prefixed_key("sk-abcdefgh", &["sk-"], 8), Some(11));
    // stops at invalid char
    assert_eq!(detect_prefixed_key("sk-abcdefgh!", &["sk-"], 8), Some(11));
    // too short
    assert!(detect_prefixed_key("sk-abc", &["sk-"], 8).is_none());
    // wrong prefix
    assert!(detect_prefixed_key("pk-abcdefgh", &["sk-"], 8).is_none());
    // multiple prefix options
    assert!(detect_prefixed_key("gsk-abcdefgh", &["gsk_", "gsk-"], 8).is_some());
    assert!(detect_prefixed_key("gsk_abcdefgh", &["gsk_", "gsk-"], 8).is_some());
}

#[test]
fn test_redact_groq_dash_style_key() {
    let input = "groq key gsk-abcdefghijklmnopqrstuvwxyz";
    let result = redact_api_keys(input);
    assert!(!result.contains("gsk-abcdefghijklmnopqrstuvwxyz"));
    assert!(result.contains("gsk-***"));
}

#[test]
fn test_detect_groq_key_valid() {
    assert!(detect_prefixed_key("gsk_abcdefghijklmno", &["gsk_", "gsk-"], 12).is_some());
}

#[test]
fn test_detect_groq_key_too_short() {
    assert!(detect_prefixed_key("gsk_abc", &["gsk_", "gsk-"], 12).is_none());
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

#[test]
fn stderr_writer_factory_produces_a_writable_redacting_sink() {
    use tracing_subscriber::fmt::MakeWriter;

    let factory = RedactingMakeWriter;
    let mut writer = factory.make_writer();
    assert_eq!(writer.write(b"").unwrap(), 0);
    writer.flush().unwrap();
}

#[test]
fn redacting_writer_flush_delegates() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = RedactingWriter { inner: &mut buf };
        w.write_all(b"hello").unwrap();
        w.flush().unwrap();
    }
    assert_eq!(String::from_utf8(buf).unwrap(), "hello");
}

#[test]
fn telegram_token_with_short_suffix_is_not_redacted() {
    let input = "token 123456789:abc";
    assert_eq!(redact_api_keys(input), input);
}

#[test]
fn redacting_writer_scrubs_keys_from_written_line() {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = RedactingWriter { inner: &mut buf };
        w.write_all(b"configured with key sk-secret-key-12345\n")
            .unwrap();
    }
    let out = String::from_utf8(buf).unwrap();
    assert!(!out.contains("sk-secret-key-12345"));
    assert!(out.contains("sk-***"));
}
