use super::*;

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
