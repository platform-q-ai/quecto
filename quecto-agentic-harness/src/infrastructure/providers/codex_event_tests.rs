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
    assert!(super::super::provider_diagnostics::packet(raw).is_none());
}

#[test]
fn s1_c123_scanner_owns_no_attacker_sized_tokens() {
    let mut failures = Vec::new();
    let mut rows = 0;
    for token in ["x".repeat(900_000), "\\u0061".repeat(140_000)] {
        for raw in [
            format!(r#"{{"type":"response.failed","response":{{"error":{{"{token}":null}}}}}}"#),
            format!(r#"{{"type":"response.failed","ignored":{{"{token}":null}}}}"#),
            format!(r#"{{"type":"response.failed","ignored":"{token}"}}"#),
            format!(r#"{{"type":{{"{token}":null}},"error":{{}}}}"#),
            format!(r#"{{"error":{{"{token}":null}}}}"#),
        ] {
            assert!(raw.len() < 1_048_576);
            let scope = crate::s1_c123_alloc::Scope::start();
            let scanned = failure_shape(&raw, Vendor::Codex);
            let (maximum, total) = scope.finish();
            assert!(scanned.is_ok());
            rows += 1;
            eprintln!(
                "C123 SCANNER row={rows} input={} maximum={maximum} total={total}",
                raw.len()
            );
            if maximum > 4096 || total > 4096 {
                failures.push((rows, maximum, total));
            }
        }
    }
    eprintln!(
        "C123 SCANNER SUMMARY selected={rows} failed={}",
        failures.len()
    );
    assert_eq!(rows, 10);
    assert!(
        failures.is_empty(),
        "private scanner allocation budget: {failures:?}"
    );
}

#[test]
fn s1_c123_strict_shape_parity() {
    strict_scanner_matches_value_strings_keys_numbers_depth_and_shapes();
    oversized_decoder_matches_actual_value_dispatch_without_error_payload();
    dispatch_trim_is_surface_specific_and_raw_semantics_remain_strict();
    for raw in [
        r#"{"\u0074ype":"response.fa\u0069led","error":{}}"#,
        r#"{"type":"response.failed","ignored":"\uDBFF\uDFFF"}"#,
        r#"{"type":"response.failed","ignored":"\uDFFF\uDBFF"}"#,
        r#"{"type":"response.failed","ignored":1e-9999}"#,
        r#"{"type":"response.failed","ignored":-18446744073709551616000000}"#,
        r#"{"type":"response.failed","ignored":0e9999}"#,
        r#"{"type":"response.failed","error":{},"type":[]}"#,
    ] {
        compare(raw);
    }
}

#[test]
fn s1_c123_generated_strict_json_differential() {
    let mut rows = 0;
    let mut check = |raw: &str| {
        compare(raw);
        let value = serde_json::from_str::<Value>(raw);
        let scanned = failure_shape(raw, Vendor::OpenAi);
        assert_eq!(scanned.is_ok(), value.is_ok(), "OpenAI validity {raw:?}");
        if let (Ok(actual), Ok(value)) = (scanned, value) {
            assert_eq!(
                actual,
                super::super::attempt_profile::is_stream_error_chunk(&value)
                    .then_some(TerminalEvent::Error),
                "OpenAI shape {raw:?}"
            );
        }
        rows += 1;
    };
    let values = [
        "null",
        "false",
        "0",
        "1e400",
        "-1e-400",
        "[]",
        "{}",
        r#""""#,
        r#""error""#,
        r#""response.failed""#,
        r#""response.incomplete""#,
        r#""\uD800""#,
        r#""\uDBFF\uDFFF""#,
        r#""世界""#,
    ];
    for left in values {
        for right in values {
            for key in ["type", "error", "ignored", "\\u0074ype"] {
                let raw = format!(r#"{{"{key}":{left},"type":{right},"error":{left}}}"#);
                check(&raw);
                // Mutations retain valid UTF-8 and exercise every byte boundary
                // in a bounded deterministic ASCII structural corpus.
                for (at, character) in raw.char_indices() {
                    let mut deleted = raw.clone();
                    deleted.replace_range(at..at + character.len_utf8(), "");
                    check(&deleted);
                }
            }
        }
    }
    for token in [
        "1".repeat(6000),
        format!("0.{}1", "0".repeat(9000)),
        format!("1{}e-9000", "0".repeat(9000)),
        format!("0e{}", "9".repeat(9000)),
    ] {
        let raw = format!(r#"{{"type":"response.failed","ignored":{token}}}"#);
        check(&raw);
        let scope = crate::s1_c123_alloc::Scope::start();
        let result = failure_shape(&raw, Vendor::Codex);
        let (maximum, total) = scope.finish();
        eprintln!(
            "C123 NUMERIC bounded maximum={maximum} total={total} valid={}",
            result.is_ok()
        );
        assert!(maximum <= 4096 && total <= 4096);
    }
    eprintln!("C123 DIFFERENTIAL SUMMARY selected={rows} failed=0");
    assert!(rows > 20_000);
}

#[test]
fn remediation_raw4097_whitespace_never_materializes_private_error_value() {
    let core = r#"{"type":"response.failed","response":{"error":{"type":"rate_limit_error"}}}"#;
    for suffix in [" ", "\t", "\r", "\n"] {
        let raw = format!("{core}{}", suffix.repeat(4097 - core.len()));
        assert_eq!(raw.len(), 4097);
        for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
            let decoded = decode(&raw, surface).unwrap();
            assert!(decoded.failed());
            assert!(
                decoded.value.is_none(),
                "raw4097 error must not own a Value"
            );
        }
    }
}

#[test]
fn remediation_in_limit_decoder_private_ownership_is_finite_all_surfaces() {
    for escaped in [false, true] {
        let message = if escaped {
            "\\u0078".repeat(640)
        } else {
            "x".repeat(3900)
        };
        let key = if escaped { "\\u006dessage" } else { "message" };
        let raw = format!(
            r#"{{"type":"response.failed","response":{{"error":{{"type":"rate_limit_error","{key}":"{message}"}}}}}}"#
        );
        assert!(raw.len() <= 4096);
        for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
            let scope = crate::s1_c123_alloc::Scope::start();
            let decoded = decode(&raw, surface).unwrap();
            let peak = scope.finish_live();
            assert!(decoded.failed());
            assert!(
                decoded.value.is_none(),
                "in-limit private error has no Value"
            );
            assert!(peak + super::super::error_fields::SCRATCH_BOUND <= 4096);
        }
    }
}
