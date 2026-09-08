use super::*;
fn hint(headers: &[(&str, &str)], maximum: u64) -> CooldownHint {
    normalize_throttle(
        429,
        headers.iter().copied(),
        UNIX_EPOCH + Duration::from_secs(784111777),
        100,
        maximum,
    )
    .unwrap()
}
#[test]
fn seconds_and_milliseconds_merge_without_retry_ceiling_assert_0() {
    assert_eq!(
        hint(&[("Retry-After", "90"), ("retry-after-ms", "1234")], 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn seconds_and_milliseconds_merge_without_retry_ceiling_assert_1() {
    assert_eq!(
        hint(&[("retry-after-ms", "1234")], 90000),
        CooldownHint::Until(1334)
    );
}
#[test]
fn seconds_and_milliseconds_merge_without_retry_ceiling_assert_2() {
    assert_eq!(
        hint(&[("retry-after", "0")], 90000),
        CooldownHint::Until(100)
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_0() {
    assert_eq!(
        hint(&[("retry-after", "-1")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_1() {
    assert_eq!(
        hint(&[("retry-after", "NaN")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_2() {
    assert_eq!(
        hint(&[("retry-after", "1.5")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_3() {
    assert_eq!(
        hint(&[("retry-after", "+1")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_4() {
    assert_eq!(
        hint(&[("retry-after", "no date")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn invalid_and_absent_require_fallback_case_5() {
    assert_eq!(hint(&[("retry-after", "")], 90000), CooldownHint::Fallback);
}
#[test]
fn invalid_and_absent_require_fallback_assert_0() {
    assert_eq!(hint(&[], 90000), CooldownHint::Fallback);
}
#[test]
fn excessive_or_overflowing_advice_is_unavailable_case_0() {
    assert_eq!(
        hint(&[("retry-after", "91"), ("retry-after-ms", "1")], 90000),
        CooldownHint::Unavailable
    );
}
#[test]
fn excessive_or_overflowing_advice_is_unavailable_case_1() {
    assert_eq!(
        hint(
            &[
                ("retry-after", "18446744073709551616"),
                ("retry-after-ms", "1")
            ],
            90000
        ),
        CooldownHint::Unavailable
    );
}
#[test]
fn excessive_or_overflowing_advice_is_unavailable_case_2() {
    assert_eq!(
        hint(
            &[
                ("retry-after", "18446744073709551615"),
                ("retry-after-ms", "1")
            ],
            90000
        ),
        CooldownHint::Unavailable
    );
}
#[test]
fn excessive_or_overflowing_advice_is_unavailable_assert_0() {
    assert_eq!(
        normalize_throttle(529, [("retry-after-ms", "1")], UNIX_EPOCH, u64::MAX, 10),
        Some(CooldownHint::Unavailable)
    );
}
#[test]
fn dates_use_receipt_pair_and_past_dates_add_zero_assert_0() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 06 Nov 1994 08:51:07 GMT")], 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn dates_use_receipt_pair_and_past_dates_add_zero_assert_1() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 06 Nov 1994 08:49:36 GMT")], 90000),
        CooldownHint::Until(100)
    );
}
#[test]
fn dates_use_receipt_pair_and_past_dates_add_zero_assert_2() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 31 Feb 1994 08:49:37 GMT")], 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn status_is_typed_not_error_prose_case_0() {
    assert_eq!(
        normalize_throttle(200, [("retry-after", "1")], UNIX_EPOCH, 0, 1000),
        None
    );
}
#[test]
fn status_is_typed_not_error_prose_case_1() {
    assert_eq!(
        normalize_throttle(400, [("retry-after", "1")], UNIX_EPOCH, 0, 1000),
        None
    );
}
#[test]
fn status_is_typed_not_error_prose_case_2() {
    assert_eq!(
        normalize_throttle(401, [("retry-after", "1")], UNIX_EPOCH, 0, 1000),
        None
    );
}
#[test]
fn status_is_typed_not_error_prose_case_3() {
    assert_eq!(
        normalize_throttle(500, [("retry-after", "1")], UNIX_EPOCH, 0, 1000),
        None
    );
}
#[test]
fn status_is_typed_not_error_prose_case_4() {
    assert_eq!(
        normalize_throttle(503, [("retry-after", "1")], UNIX_EPOCH, 0, 1000),
        None
    );
}
#[test]
fn status_is_typed_not_error_prose_assert_0() {
    assert_eq!(
        normalize_throttle(529, [], UNIX_EPOCH, 0, 1),
        Some(CooldownHint::Fallback)
    );
}
#[test]
fn status_is_typed_not_error_prose_assert_1() {
    assert_eq!(hint(&[], 0), CooldownHint::Unavailable);
}
#[test]
fn reversed_units_preserve_longest_hint_assert_0() {
    assert_eq!(
        hint(&[("retry-after-ms", "1234"), ("retry-after", "90")], 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn repeated_headers_preserve_longest_hint_in_either_order_case_0() {
    assert_eq!(
        hint(&[("retry-after", "2"), ("retry-after", "9")], 90000),
        CooldownHint::Until(9100)
    );
}
#[test]
fn repeated_headers_preserve_longest_hint_in_either_order_case_1() {
    assert_eq!(
        hint(&[("retry-after", "9"), ("retry-after", "2")], 90000),
        CooldownHint::Until(9100)
    );
}
#[test]
fn invalid_hint_does_not_hide_valid_hint_in_either_order_case_0() {
    assert_eq!(
        hint(&[("retry-after", "invalid"), ("retry-after", "9")], 90000),
        CooldownHint::Until(9100)
    );
}
#[test]
fn invalid_hint_does_not_hide_valid_hint_in_either_order_case_1() {
    assert_eq!(
        hint(&[("retry-after", "9"), ("retry-after", "invalid")], 90000),
        CooldownHint::Until(9100)
    );
}
#[test]
fn excessive_last_hint_fails_closed_assert_0() {
    assert_eq!(
        hint(&[("retry-after-ms", "1"), ("retry-after", "91")], 90000),
        CooldownHint::Unavailable
    );
}
#[test]
fn millisecond_header_name_is_case_insensitive_assert_0() {
    assert_eq!(
        hint(&[("ReTrY-AfTeR-Ms", "1234")], 90000),
        CooldownHint::Until(1334)
    );
}
#[test]
fn milliseconds_accept_exact_limit_assert_0() {
    assert_eq!(
        hint(&[("retry-after-ms", "90000")], 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn milliseconds_reject_one_above_limit_assert_0() {
    assert_eq!(
        hint(&[("retry-after-ms", "90001")], 90000),
        CooldownHint::Unavailable
    );
}
#[test]
fn date_accepts_exact_limit_assert_0() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 06 Nov 1994 08:51:07 GMT")], 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn date_rejects_one_millisecond_above_limit_assert_0() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 06 Nov 1994 08:51:07 GMT")], 89999),
        CooldownHint::Unavailable
    );
}
#[test]
fn representable_seconds_with_overflowing_millisecond_product_fail_closed_assert_0() {
    assert_eq!(
        hint(&[("retry-after", "18446744073709552")], u64::MAX),
        CooldownHint::Unavailable
    );
}
#[test]
fn current_date_adds_zero_delay_assert_0() {
    assert_eq!(
        hint(&[("retry-after", "Sun, 06 Nov 1994 08:49:37 GMT")], 90000),
        CooldownHint::Until(100)
    );
}
#[test]
fn zero_milliseconds_add_zero_delay_assert_0() {
    assert_eq!(
        hint(&[("retry-after-ms", "0")], 90000),
        CooldownHint::Until(100)
    );
}

#[test]
fn captured_wall_time_is_not_reread_for_date_deadline() {
    // An old, deterministic receipt timestamp makes consulting current wall time
    // visibly wrong: the absolute deadline must still be receipt monotonic+90s.
    let received = UNIX_EPOCH + Duration::from_secs(784111777);
    assert_eq!(
        normalize_throttle(
            429,
            [("retry-after", "Sun, 06 Nov 1994 08:51:07 GMT")],
            received,
            500,
            90000
        ),
        Some(CooldownHint::Until(90500))
    );
}
#[test]
fn unsupported_status_ignores_even_valid_long_hint() {
    assert_eq!(
        normalize_throttle(401, [("retry-after", "90")], UNIX_EPOCH, 100, 90000),
        None
    );
}

// HTTP-date format and validation boundaries.
fn date_hint(value: &str, offset_ns: u64, maximum: u64) -> CooldownHint {
    normalize_throttle(
        429,
        [("retry-after", value)],
        UNIX_EPOCH + Duration::from_secs(784111777) + Duration::from_nanos(offset_ns),
        100,
        maximum,
    )
    .unwrap()
}
#[test]
fn rfc850_future_date() {
    assert_eq!(
        date_hint("Sunday, 06-Nov-94 08:51:07 GMT", 0, 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn asctime_future_date() {
    assert_eq!(
        date_hint("Sun Nov  6 08:51:07 1994", 0, 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn rfc850_past_date_is_zero_not_fallback() {
    assert_eq!(
        date_hint("Sunday, 06-Nov-94 08:49:36 GMT", 0, 90000),
        CooldownHint::Until(100)
    );
}
#[test]
fn asctime_current_date_is_zero_not_fallback() {
    assert_eq!(
        date_hint("Sun Nov  6 08:49:37 1994", 0, 90000),
        CooldownHint::Until(100)
    );
}
#[test]
fn fraction_of_second_numeric_advice_is_invalid() {
    assert_eq!(date_hint("1.5", 0, 90000), CooldownHint::Fallback);
}
#[test]
fn fractional_millisecond_numeric_advice_is_invalid() {
    assert_eq!(
        normalize_throttle(429, [("retry-after-ms", "1.5")], UNIX_EPOCH, 100, 90000),
        Some(CooldownHint::Fallback)
    );
}
#[test]
fn fractional_http_date_is_invalid() {
    assert_eq!(
        date_hint("Sun, 06 Nov 1994 08:51:07.5 GMT", 0, 90000),
        CooldownHint::Fallback
    );
}
#[test]
fn mismatched_weekday_is_invalid_in_every_form() {
    for date in [
        "Mon, 06 Nov 1994 08:51:07 GMT",
        "Monday, 06-Nov-94 08:51:07 GMT",
        "Mon Nov  6 08:51:07 1994",
    ] {
        assert_eq!(date_hint(date, 0, 90000), CooldownHint::Fallback, "{date}");
    }
}
#[test]
fn impossible_calendar_date_is_invalid_in_every_form() {
    for date in [
        "Sun, 31 Feb 1994 08:49:37 GMT",
        "Sunday, 31-Feb-94 08:49:37 GMT",
        "Sun Feb 31 08:49:37 1994",
    ] {
        assert_eq!(date_hint(date, 0, 90000), CooldownHint::Fallback, "{date}");
    }
}
#[test]
fn fractional_receipt_rounds_up_without_early_retry() {
    assert_eq!(
        date_hint("Sun, 06 Nov 1994 08:51:07 GMT", 1, 90000),
        CooldownHint::Until(90100)
    );
}
#[test]
fn fractional_receipt_does_not_round_excess_into_policy() {
    assert_eq!(
        date_hint("Sun, 06 Nov 1994 08:51:07 GMT", 1, 89999),
        CooldownHint::Unavailable
    );
}
#[test]
fn exact_millisecond_receipt_preserves_precision() {
    assert_eq!(
        date_hint("Sun, 06 Nov 1994 08:51:07 GMT", 1000000, 89999),
        CooldownHint::Until(90099)
    );
}

#[test]
fn root_error_envelope_throttle_code_is_recognized() {
    assert!(is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "rate_limit_exceeded", "message": "Request failed"
    })));
}

#[test]
fn root_billing_code_overrides_direct_nested_throttle() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "insufficient_quota",
        "error": {"type": "rate_limit_error"}
    })));
}

#[test]
fn root_billing_code_overrides_responses_nested_throttle() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "billing_hard_limit_reached",
        "response": {"error": {"type": "overloaded_error"}}
    })));
}

#[test]
fn direct_nested_billing_overrides_root_throttle() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "rate_limit_exceeded",
        "error": {"code": "insufficient_quota"}
    })));
}

#[test]
fn responses_nested_billing_overrides_root_throttle() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "rate_limit_exceeded",
        "response": {"error": {"code": "billing_hard_limit_reached"}}
    })));
}

#[test]
fn arbitrary_root_code_without_error_envelope_is_not_throttle() {
    assert!(!is_typed_throttle(
        &serde_json::json!({"code": "rate_limit_exceeded"})
    ));
}

#[test]
fn success_event_root_code_is_not_throttle() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "response.output_text.delta", "code": "rate_limit_exceeded"
    })));
}

#[test]
fn root_throttle_type_without_error_envelope_is_not_confirmation() {
    assert!(!is_typed_throttle(
        &serde_json::json!({"type": "rate_limit_error"})
    ));
}

#[test]
fn root_error_prose_is_not_throttle_confirmation() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "message": "429 rate_limit_exceeded overloaded_error"
    })));
}

#[test]
fn root_error_unknown_code_is_not_throttle_confirmation() {
    assert!(!is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "invalid_request_error", "message": "rate_limit_exceeded"
    })));
}

#[test]
fn root_error_billing_prose_does_not_override_structured_throttle() {
    assert!(is_typed_throttle(&serde_json::json!({
        "type": "error", "code": "rate_limit_exceeded", "message": "insufficient_quota"
    })));
}

#[test]
fn arbitrary_root_billing_code_does_not_override_nested_throttle() {
    assert!(is_typed_throttle(&serde_json::json!({
        "code": "insufficient_quota", "error": {"type": "rate_limit_error"}
    })));
}
