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
        r#"{"type":"response.output_item.added","item":{"type":"function_call","name":"f"}}"#,
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
        r#"{"type":"content_block_start","content_block":{"type":"tool_use","name":"f"}}"#,
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
        (r#"{"tool_calls":[{"index":0}]}"#, "null"),
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
