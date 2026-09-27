use super::*;
use serde_json::json;

#[test]
fn a_codex_delta_counts_only_its_generated_text() {
    for kind in RESPONSES_OUTPUT_DELTAS {
        let event = json!({"type": kind, "delta": "héllo", "item_id": "x", "sequence_number": 7});
        assert_eq!(output_bytes(Vendor::Codex, &event), 6, "{kind}");
    }
}

#[test]
fn a_codex_event_that_is_no_output_delta_counts_nothing() {
    for event in [
        json!({"type": "response.created", "response": {}}),
        json!({"type": "response.output_text.done", "text": "hello"}),
        json!({"type": "response.function_call_arguments.done", "arguments": "{}"}),
        json!({"type": "response.output_item.added", "item": {"type": "function_call"}}),
        json!({"type": "response.output_text.delta", "delta": 5}),
        json!({"type": "response.audio.delta", "delta": "not model text output"}),
        json!({"type": "response.output_text.annotation.delta", "delta": "x"}),
        json!({"delta": "untyped"}),
    ] {
        assert_eq!(output_bytes(Vendor::Codex, &event), 0, "{event}");
    }
}

#[test]
fn every_codex_output_delta_is_a_known_event() {
    for kind in RESPONSES_OUTPUT_DELTAS {
        assert!(
            super::super::attempt_events::KNOWN_EVENTS.contains(kind),
            "{kind}"
        );
    }
}

#[test]
fn an_openai_chunk_counts_content_reasoning_refusal_and_tool_arguments() {
    let event = json!({"choices": [
        {"index": 0, "delta": {"content": "ab", "reasoning": "cde", "reasoning_content": "f",
                               "refusal": "gh"}},
        {"index": 1, "delta": {"tool_calls": [
            {"index": 0, "function": {"name": "grep", "arguments": "{\"q\":"}},
            {"index": 1, "function": {"arguments": "1}"}}
        ]}}
    ]});
    // `reasoning` is the reasoning; `reasoning_content` its alternative name.
    assert_eq!(output_bytes(Vendor::OpenAi, &event), 2 + 3 + 2 + 5 + 2);
}

/// #2210 review: the reasoning is counted once, under whichever name the
/// provider streams it, as the handler reads it.
#[test]
fn openai_reasoning_is_counted_under_either_name_once() {
    let cases = [
        (json!({"reasoning": "abcd"}), 4),
        (json!({"reasoning_content": "abc"}), 3),
        (json!({"reasoning": "ab", "reasoning_content": "abcdef"}), 2),
        (json!({"reasoning": null, "reasoning_content": "abcdef"}), 0),
    ];
    for (delta, bytes) in cases {
        let event = json!({"choices": [{"index": 0, "delta": delta}]});
        assert_eq!(output_bytes(Vendor::OpenAi, &event), bytes, "{event}");
    }
}

#[test]
fn an_openai_chunk_without_output_counts_nothing() {
    for event in [
        json!({"choices": [{"index": 0, "delta": {"role": "assistant"}}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"completion_tokens": 9}}),
        json!({"choices": "none"}),
        json!({"error": {"message": "boom"}}),
    ] {
        assert_eq!(output_bytes(Vendor::OpenAi, &event), 0, "{event}");
    }
}

#[test]
fn an_anthropic_delta_counts_text_thinking_and_tool_input() {
    let cases = [
        (
            json!({"index": 0, "delta": {"type": "text_delta", "text": "hello"}}),
            5,
        ),
        (
            json!({"index": 0, "delta": {"type": "thinking_delta", "thinking": "hmm"}}),
            3,
        ),
        (
            json!({"index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"a\""}}),
            4,
        ),
        (
            json!({"index": 0, "delta": {"type": "signature_delta", "signature": "sig"}}),
            0,
        ),
        (
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
            0,
        ),
        (json!({"type": "ping"}), 0),
    ];
    for (event, bytes) in cases {
        assert_eq!(output_bytes(Vendor::Anthropic, &event), bytes, "{event}");
    }
}
