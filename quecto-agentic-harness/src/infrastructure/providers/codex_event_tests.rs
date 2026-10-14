use super::*;
use serde_json::Value;

fn compare(raw: &str) {
    let actual = serde_json::from_str::<Value>(raw);
    let strict = failure_shape(raw, Vendor::Codex);
    assert_eq!(strict.is_ok(), actual.is_ok(), "strict acceptance: {raw}");
    if let (Ok(strict), Ok(value)) = (strict, actual) {
        let expected = terminal(&value).filter(|terminal| {
            matches!(
                terminal,
                TerminalEvent::Error
                    | TerminalEvent::ResponseFailed
                    | TerminalEvent::ResponseIncomplete
            )
        });
        assert_eq!(strict, expected, "strict dispatch: {raw}");
    }
}
#[test]
fn strict_scanner_matches_value_strings_keys_numbers_depth_and_shapes() {
    let values = [
        r#"null"#,
        r#"false"#,
        r#"42"#,
        r#"-0.0"#,
        r#"[]"#,
        r#"[{},null,1]"#,
        r#""primitive""#,
        r#"{"type":null,"error":{}}"#,
        r#"{"type":[],"error":[]}"#,
        r#"{"type":{},"error":false}"#,
        r#"{"type":"response.failed","response":{"error":{"message":"\uD800"}}}"#,
        r#"{"type":"response.failed","response":{"error":{"message":"\uDC00"}}}"#,
        r#"{"type":"response.failed","ignored":{"\uD800":1}}"#,
        r#"{"type":"response.failed","ignored":{"k":"\uD800\uDC00"}}"#,
        r#"{"type":"response.failed","ignored":{"k":"\uD800x"}}"#,
        r#"{"type":"response.failed","ignored":["\q"]}"#,
        r#"{"type":"response.failed","ignored":[{"number":1e400}]}"#,
        r#"{"type":"response.failed","ignored":[{"number":18446744073709551616}]}"#,
        r#"{"type":"response.failed","ignored":-1e400}"#,
        r#"{"type":"response.failed","ignored":01}"#,
        r#"{"type":"response.failed","ignored":1.}"#,
        r#"{"type":"response.failed","type":"response.completed","error":{}}"#,
        r#"{"type":"response.completed","type":"response.failed","error":null}"#,
        r#"{"type":null,"error":{},"error":[]}"#,
        r#"{"type":null,"error":[],"error":{}}"#,
        r#"{"type":[{"bad":"\uD800"}],"error":{}}"#,
        r#"{"type":null,"error":[{"bad":"\uD800"}]}"#,
        r#"{"type":null,"error":"\uD800"}"#,
        r#"{"type":"response.failed"} trailing"#,
    ];
    for raw in values {
        compare(raw);
    }
    for depth in [1, 32, 125, 126, 127, 128, 129, 150] {
        let nested = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        compare(&nested);
        compare(&format!(
            "{{\"type\":\"response.failed\",\"ignored\":{nested}}}"
        ));
        compare(&format!("{{\"type\":{nested},\"error\":{{}}}}"));
    }
}
#[test]
fn oversized_decoder_matches_actual_value_dispatch_without_error_payload() {
    for suffix in [r#"\uD800"#, r#"\uD800\uDC00"#, "normal"] {
        let raw = format!(
            r#"{{"type":"response.failed","ignored":"{}{suffix}"}}"#,
            "p".repeat(4200)
        );
        let value = serde_json::from_str::<Value>(&raw);
        let decoded = decode(&raw, Surface::Incremental);
        assert_eq!(decoded.is_ok(), value.is_ok());
        if let Ok(decoded) = decoded {
            assert!(decoded.failed());
            assert!(decoded.value.is_none(), "oversized error retains no Value");
        }
    }
    for raw in [
        format!(r#""{}""#, "p".repeat(6000)),
        format!(
            r#"{{"type":"response.output_text.delta","delta":"{}"}}"#,
            "t".repeat(6000)
        ),
    ] {
        let decoded = decode(&raw, Surface::Incremental).unwrap();
        assert!(!decoded.failed());
        assert!(
            decoded.value.is_some(),
            "ordinary large output stays deliverable"
        );
    }
}
#[test]
fn dispatch_trim_is_surface_specific_and_raw_semantics_remain_strict() {
    let raw = "{\"type\":\"response.failed\"}\u{2003}";
    assert!(decode(raw, Surface::Assembled).unwrap().failed());
    assert!(decode(raw, Surface::Chat).unwrap().failed());
    assert!(decode(raw, Surface::Incremental).is_err());
    assert!(super::super::provider_diagnostics::raw_json(raw).is_none());
}
