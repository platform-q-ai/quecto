use super::*;
#[test]
fn remediation_finite_projection_selected_duplicates_escapes_and_ignored_paths() {
    let cases = [
        r#"{"error":{"type":"rate_limit_error"},"error":null,"response":{"error":{"type":"invalid_request_error"}}}"#,
        r#"{"error":{},"response":{"error":{"type":"rate_limit_error"}}}"#,
        r#"{"response":{"error":{"type":"rate_limit_error"}},"response":null}"#,
        r#"{"response":{"error":{"type":"rate_limit_error"},"error":null}}"#,
        r#"{"response":{"error":{"type":"rate_limit_error"}},"ignored":{"response":{"error":{"type":"insufficient_quota"}}}}"#,
        r#"{"type":"response.failed","error":{"type":"rate_limit_error"},"response":{"error":{"type":"invalid_request_error"}}}"#,
        r#"{"\u0074ype":"response.fa\u0069led","response":{"error":{"\u0074ype":"rate_limit_\u0065rror","code":429}}}"#,
    ];
    for raw in cases {
        let packet = parse(raw).unwrap();
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let selected = super::super::provider_diagnostics::stream_error_envelope(&value);
        assert_eq!(
            packet.stream().names(),
            value_fields(selected).names(),
            "{raw}"
        );
        let direct = value.get("error").filter(|v| v.is_object());
        let nested = value
            .get("response")
            .and_then(|v| v.get("error"))
            .filter(|v| v.is_object());
        assert_eq!(
            packet.http().names(),
            value_fields(direct.or(nested).unwrap_or(&value)).names(),
            "{raw}"
        );
    }
}
#[test]
fn remediation_finite_predicates_escaped_prose_and_incomplete_replacement() {
    for message in [
        "model is not supported when using Codex with a ChatGPT account",
        "mismatched model is not supported when using Codex with a ChatGPT account and trailing",
    ] {
        let raw = serde_json::json!({"error":{"message":message}})
            .to_string()
            .replace('m', "\\u006d");
        assert!(parse(&raw).unwrap().http().message_refusal);
    }
    for (raw, expected) in [
        (
            r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#,
            Some("max_output_tokens"),
        ),
        (
            r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"},"incomplete_details":null}}"#,
            None,
        ),
        (
            r#"{"response":{"incomplete_details":{"reason":"max_output_tokens"}},"response":{}}"#,
            None,
        ),
    ] {
        let packet = parse(raw).unwrap();
        assert_eq!(
            NAMES
                .get(usize::from(packet.reason))
                .copied()
                .filter(|s| !s.is_empty()),
            expected
        );
    }
}
