//! #2433: which events carry output, on each wire.
use super::carries_output;

#[test]
fn responses_events_carry_output_by_what_they_hold() {
    let output = [
        r#"{"type":"response.output_text.delta","delta":"a"}"#,
        r#"{"type":"response.function_call_arguments.delta","delta":"{"}"#,
        r#"{"type":"response.refusal.delta","delta":"no"}"#,
        r#"{"type":"response.reasoning_summary_text.delta","delta":"hm"}"#,
        r#"{"type":"response.reasoning_text.delta","delta":"hm"}"#,
        r#"{"type":"response.output_text.done","text":"ab"}"#,
        r#"{"type":"response.function_call_arguments.done","arguments":"{}"}"#,
        r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"ab"}]}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","summary":[{"type":"summary_text","text":"hm"}]}}"#,
        r#"{"type":"response.content_part.done","part":{"type":"output_text","text":"ab"}}"#,
        r#"{"type":"response.reasoning_summary_part.done","part":{"type":"summary_text","text":"hm"}}"#,
        r#"{"type":"response.completed","response":{}}"#,
        r#"{"type":"response.incomplete","response":{}}"#,
        r#"{"type":"response.failed","response":{}}"#,
        r#"{"type":"error","message":"x"}"#,
    ];
    for data in output {
        assert!(carries_output(data), "{data}");
    }
    let none = [
        r#"{"type":"response.created","response":{}}"#,
        r#"{"type":"response.in_progress","response":{}}"#,
        r#"{"type":"response.output_text.delta","delta":""}"#,
        r#"{"type":"response.reasoning_summary_text.delta","delta":""}"#,
        r#"{"type":"response.output_item.added","item":{"type":"message","content":[]}}"#,
        r#"{"type":"response.output_item.added","item":{"type":"reasoning","summary":[]}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"reasoning","summary":[],"encrypted_content":"x"}}"#,
        r#"{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":""}]}}"#,
        r#"{"type":"response.content_part.added","part":{"type":"output_text","text":""}}"#,
        r#"{"type":"response.reasoning_summary_part.added","part":{"type":"summary_text","text":""}}"#,
        r#"{"type":"response.output_text.done","text":""}"#,
        r#"{"type":"made.up"}"#,
        "{not json",
        "",
    ];
    for data in none {
        assert!(!carries_output(data), "{data}");
    }
}

#[test]
fn messages_events_carry_output_by_what_they_hold() {
    let output = [
        r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        r#"{"type":"message_stop"}"#,
    ];
    for data in output {
        assert!(carries_output(data), "{data}");
    }
    let none = [
        r#"{"type":"ping"}"#,
        r#"{"type":"message_start","message":{"content":[]}}"#,
        r#"{"type":"content_block_start","content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":""}}"#,
        r#"{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"s"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":null},"usage":{}}"#,
    ];
    for data in none {
        assert!(!carries_output(data), "{data}");
    }
}

#[test]
fn chat_chunks_carry_output_by_what_they_hold() {
    let chunk = |delta: &str, finish: &str| {
        format!(r#"{{"choices":[{{"index":0,"delta":{delta},"finish_reason":{finish}}}]}}"#)
    };
    for (delta, finish) in [
        (r#"{"content":"a"}"#, "null"),
        (r#"{"reasoning_content":"hm"}"#, "null"),
        (r#"{"reasoning":"hm"}"#, "null"),
        (r#"{"refusal":"no"}"#, "null"),
        (
            r#"{"tool_calls":[{"index":0,"function":{"name":"f"}}]}"#,
            "null",
        ),
        ("{}", r#""stop""#),
    ] {
        assert!(carries_output(&chunk(delta, finish)), "{delta} {finish}");
    }
    assert!(carries_output("[DONE]"));
    for (delta, finish) in [
        (r#"{"content":""}"#, "null"),
        (r#"{"role":"assistant"}"#, "null"),
        (r#"{"tool_calls":[]}"#, "null"),
        ("{}", "null"),
    ] {
        assert!(!carries_output(&chunk(delta, finish)), "{delta} {finish}");
    }
    assert!(!carries_output(r#"{"choices":[]}"#));
}

/// #2433 review L5: a tool call is output once it names a function or
/// carries arguments; an empty one, and a Responses or Messages call just
/// opened, are not.
#[test]
fn an_empty_or_just_opened_tool_call_is_no_output() {
    let none = [
        r#"{"type":"response.output_item.added","item":{"type":"function_call","name":"f","arguments":""}}"#,
        r#"{"type":"response.function_call_arguments.delta","delta":""}"#,
        r#"{"type":"content_block_start","content_block":{"type":"tool_use","name":"f","input":{}}}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"","arguments":""}}]}}]}"#,
    ];
    for data in none {
        assert!(!carries_output(data), "{data}");
    }
    let output = [
        r#"{"type":"response.output_item.done","item":{"type":"function_call","name":"f","arguments":"{}"}}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"f"}}]}}]}"#,
        r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{"}}]}}]}"#,
    ];
    for data in output {
        assert!(carries_output(data), "{data}");
    }
}

/// #2433 review M4: a long event is output by its type alone.
#[test]
fn a_long_event_is_output_by_the_type_its_start_names() {
    use super::long_event_carries_output as long;
    let output = [
        r#"{"type":"response.output_text.delta","delta":"xxxx"#,
        r#"{"type":"response.output_item.done","item":{"content":[{"text":"xx"#,
        r#"{"type":"response.completed","response":{"output":[{"xx"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"xx"#,
        r#"{"index":0,"delta":{"type":"text_delta","text":"xx"#,
        r#"{"choices":[{"index":0,"delta":{"content":"xx"#,
    ];
    for start in output {
        assert!(long(start.as_bytes()), "{start}");
    }
    let none = [
        r#"{"type":"response.in_progress","response":{"padding":"xx"#,
        r#"{"type":"response.created","response":{"xx"#,
        r#"{"type": "ping", "padding":"xx"#,
        r#"{"padding":"xx"#,
        "",
    ];
    for start in none {
        assert!(!long(start.as_bytes()), "{start}");
    }
}

/// #2433 review round 3 (N1): a chat chunk too long to read whole is
/// output when it carries content or a tool call, though a `"type"` comes
/// later inside its choices.
#[test]
fn a_long_chat_chunk_is_output_by_what_its_choices_carry() {
    use super::long_event_carries_output as long;
    let call = format!(
        r#"{{"choices":[{{"index":0,"delta":{{"tool_calls":[{{"index":0,"id":"c","type":"function","function":{{"name":"f","arguments":"{}"#,
        "x".repeat(70 * 1024)
    );
    assert!(long(&call.as_bytes()[..64 * 1024]));
    let padded = r#"{"choices":[{"index":0,"delta":{"role":"assistant","padding":"xxx"#;
    assert!(!long(padded.as_bytes()));
    // A start cut inside a character is read up to it.
    let cut = "{\"type\":\"response.output_text.delta\",\"delta\":\"é".as_bytes();
    assert!(long(&cut[..cut.len() - 1]));
}
