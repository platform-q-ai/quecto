use super::*;

#[test]
fn test_cost_calculation_sonnet_4() {
    let usage = UsageInfo {
        prompt_tokens: 1000,
        completion_tokens: 500,
        cache_read_tokens: Some(200),
        cache_write_tokens: Some(100),
        context_tokens: None,
        cost: None,
    };
    let pricing = model_pricing("claude-sonnet-4-6").unwrap();
    let cost = pricing.cost_for(&usage);
    // Input: 1000/1M * $3.00 = $0.003 = 3000 micro-USD
    assert_eq!(cost.input_cost_micro_usd, 3000);
    assert!((cost.input_cost_usd() - 0.003).abs() < 1e-9);
    // Output: 500/1M * $15.00 = $0.0075 = 7500 micro-USD
    assert_eq!(cost.output_cost_micro_usd, 7500);
    assert!((cost.output_cost_usd() - 0.0075).abs() < 1e-9);
    // Cache read: 200/1M * $0.30 = $0.00006 = 60 micro-USD (integer: 200*300_000/1_000_000=60)
    assert_eq!(cost.cache_read_cost_micro_usd, 60);
    // Cache write: 100/1M * $3.75 = $0.000375 = 375 micro-USD (100*3_750_000/1_000_000=375)
    assert_eq!(cost.cache_write_cost_micro_usd, 375);
    // Total
    assert_eq!(cost.total_cost_micro_usd, 3000 + 7500 + 60 + 375);
}

#[test]
fn test_cost_calculation_opus_4() {
    let usage = UsageInfo {
        prompt_tokens: 1_000_000,
        completion_tokens: 100_000,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    };
    let pricing = model_pricing("claude-opus-4-6").unwrap();
    let cost = pricing.cost_for(&usage);
    // Opus 4.5/4.6: $5.00/MTok input (not $15 — that was Opus 4.1 and earlier)
    // Input: 1M/1M * $5.00 = $5.00 = 5_000_000 micro-USD
    assert_eq!(cost.input_cost_micro_usd, 5_000_000);
    assert!((cost.input_cost_usd() - 5.0).abs() < 1e-6);
    // Output: 100K/1M * $25.00 = $2.50 = 2_500_000 micro-USD
    assert_eq!(cost.output_cost_micro_usd, 2_500_000);
    assert!((cost.output_cost_usd() - 2.5).abs() < 1e-6);
}

#[test]
fn test_cost_calculation_sonnet_5_flat_pricing() {
    let usage = UsageInfo {
        prompt_tokens: 1_000_000,
        completion_tokens: 100_000,
        cache_read_tokens: Some(1_000_000),
        cache_write_tokens: Some(1_000_000),
        context_tokens: None,
        cost: None,
    };
    let pricing = model_pricing("claude-sonnet-5").unwrap();
    let cost = pricing.cost_for(&usage);
    assert_eq!(cost.input_cost_micro_usd, 3_000_000);
    assert_eq!(cost.output_cost_micro_usd, 1_500_000);
    assert_eq!(cost.cache_read_cost_micro_usd, 300_000);
    assert_eq!(cost.cache_write_cost_micro_usd, 3_750_000);
    assert_eq!(cost.total_cost_micro_usd, 8_550_000);
}

#[test]
fn test_model_pricing_unknown_returns_none() {
    assert!(model_pricing("gpt-4o").is_none());
    assert!(model_pricing("unknown-model").is_none());
    assert!(model_pricing("claude-3-5-sonnet-20241022").is_none());
    assert!(model_pricing("claude-3-5-haiku-20241022").is_none());
    assert!(model_pricing("claude-3-7-sonnet-20250219").is_none());
}

#[test]
fn test_model_pricing_known_models() {
    assert!(model_pricing("claude-sonnet-5").is_some());
    assert!(model_pricing("claude-sonnet-5-20260630").is_some());
    assert!(model_pricing("claude-sonnet-4-6").is_some());
    assert!(model_pricing("claude-opus-4-6").is_some());
    assert!(model_pricing("claude-haiku-4-5").is_some());
    assert!(model_pricing("claude-haiku-4-5-20251001").is_some());
    assert!(model_pricing("gpt-6-astra").is_some());
    assert!(model_pricing("gpt-6-sol").is_some());
    assert!(model_pricing("gpt-6-luna").is_some());
    assert!(model_pricing("gpt-5.6-sol").is_some());
    assert!(model_pricing("gpt-5.6-terra").is_some());
    assert!(model_pricing("gpt-5.6-luna").is_some());
    // Prefix match covers dated variants of all three supported families
    assert!(model_pricing("claude-sonnet-4-6").is_some());
    assert!(model_pricing("claude-opus-4-20250514").is_some());
    assert!(model_pricing("claude-haiku-4-20250514").is_some());
    // Case-insensitive
    assert!(model_pricing("Claude-Sonnet-4-6").is_some());
    assert!(model_pricing("CLAUDE-OPUS-4-6").is_some());
    assert!(model_pricing("Claude-Haiku-4-5").is_some());
}

#[test]
fn test_starts_with_ci() {
    assert!(starts_with_ci("Claude-Sonnet-4-6", "claude-sonnet-4"));
    assert!(starts_with_ci("CLAUDE-OPUS-4", "claude-opus-4"));
    assert!(!starts_with_ci("claude-3-5-sonnet", "claude-sonnet-4"));
    assert!(!starts_with_ci("short", "claude-sonnet-4"));
}

#[test]
fn stop_reason_parse_all_variants() {
    assert_eq!(StopReason::parse("end_turn"), StopReason::EndTurn);
    assert_eq!(StopReason::parse("max_tokens"), StopReason::MaxTokens);
    assert_eq!(
        StopReason::parse("model_context_window_exceeded"),
        StopReason::MaxTokens
    );
    assert_eq!(StopReason::parse("tool_use"), StopReason::ToolUse);
    assert_eq!(StopReason::parse("refusal"), StopReason::Refusal);
    assert_eq!(StopReason::parse("pause_turn"), StopReason::EndTurn);
    assert_eq!(StopReason::parse("stop_sequence"), StopReason::EndTurn);
    assert_eq!(StopReason::parse("sensitive"), StopReason::Error);
    assert_eq!(StopReason::parse("error"), StopReason::Error);
    assert_eq!(StopReason::parse("aborted"), StopReason::Aborted);
    assert_eq!(
        StopReason::parse("custom_stop"),
        StopReason::Unknown("custom_stop".into())
    );
}

#[test]
fn stop_reason_as_str_and_display_round_trip() {
    assert_eq!(StopReason::EndTurn.as_str(), "end_turn");
    assert_eq!(StopReason::MaxTokens.as_str(), "max_tokens");
    assert_eq!(StopReason::ToolUse.as_str(), "tool_use");
    assert_eq!(StopReason::Refusal.as_str(), "refusal");
    assert_eq!(StopReason::Error.as_str(), "error");
    assert_eq!(StopReason::Aborted.as_str(), "aborted");
    assert_eq!(StopReason::Unknown("weird".into()).as_str(), "weird");
    assert_eq!(StopReason::ToolUse.to_string(), "tool_use");
}

/// #2116: OpenAI Chat Completions `finish_reason` values.
#[test]
fn openai_finish_reasons_map_to_stop_reasons() {
    for (raw, expected) in [
        ("stop", StopReason::EndTurn),
        ("length", StopReason::MaxTokens),
        ("tool_calls", StopReason::ToolUse),
        ("function_call", StopReason::ToolUse),
        ("content_filter", StopReason::Refusal),
        ("error", StopReason::Error),
        ("model_context_window_exceeded", StopReason::MaxTokens),
        ("other", StopReason::Unknown("other".into())),
    ] {
        let live = StopReason::from_openai_finish_reason(raw);
        assert_eq!(live, expected, "{raw}");
        // What a session reload reads back is what was live.
        assert_eq!(StopReason::parse(live.as_str()), live, "{raw} round-trip");
    }
}

// ── #2123: only a JSON object is a valid argument set ─────────────────────

fn call_with(arguments: &str) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: "bash".into(),
        arguments: arguments.into(),
    }
}

#[test]
fn tool_call_arguments_are_classified_as_object_empty_or_invalid() {
    assert_eq!(
        call_with(r#"{"command":"ls"}"#).argument_shape(),
        ToolArguments::Object(r#"{"command":"ls"}"#)
    );
    for empty in ["", "   ", "\n"] {
        assert_eq!(call_with(empty).argument_shape(), ToolArguments::Empty);
    }
    // Truncated at an output limit, a bare array, string or number: invalid.
    for invalid in [
        r#"{"command":"ls -la /very/lo"#,
        "[1,2]",
        r#""ls""#,
        "42",
        "null",
        "{,}",
    ] {
        assert_eq!(
            call_with(invalid).argument_shape(),
            ToolArguments::Invalid(invalid),
            "{invalid}"
        );
    }
}

#[test]
fn only_a_json_object_goes_on_the_wire_so_a_bad_call_never_poisons_history() {
    assert_eq!(
        call_with(r#"{"command":"ls"}"#).wire_arguments(),
        r#"{"command":"ls"}"#
    );
    for other in ["", r#"{"command":"ls -la /very/lo"#, "[1,2]", "null"] {
        assert_eq!(call_with(other).wire_arguments(), "{}", "{other}");
    }
}

#[test]
fn only_json_whitespace_is_ignored_around_an_object() {
    // A non-breaking space would survive to the provider and poison history.
    let nbsp = "\u{00A0}{\"a\":1}";
    assert_eq!(
        call_with(nbsp).argument_shape(),
        ToolArguments::Invalid(nbsp)
    );
    assert_eq!(call_with(nbsp).wire_arguments(), "{}");
    let spaced = " \n{\"a\":1}\t";
    assert_eq!(
        call_with(spaced).argument_shape(),
        ToolArguments::Object(spaced)
    );
}
