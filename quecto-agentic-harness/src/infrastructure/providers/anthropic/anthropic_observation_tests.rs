//! #2597: what the attempt observer makes of the Messages (Anthropic) wire, pinned
//! before attempt observation moved out of `transport/` into per-vendor
//! strategies: every case must read exactly as it did.
use crate::domain::inference::value_objects::attempt_diagnostics::{TerminalEvent, Termination};
use crate::infrastructure::providers::transport::attempt_profile::{Profile, Surface};
use crate::infrastructure::providers::transport::attempt_transport::{observation, observed};

fn profile() -> Profile {
    Profile::new(
        &super::anthropic_wire::ANTHROPIC_WIRE,
        Surface::Incremental,
        Default::default(),
    )
}

/// Each case: its name, the SSE lines, and the observation they make.
const CASES: &[(&str, &[&str], &str)] = &[
    (
        "tool_text_thinking_and_stop",
        &[
            r#"event: message_start"#,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":3}}}"#,
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hi"}}"#,
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t","name":"read","input":{}}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"p\""}}"#,
            r#"event: content_block_stop"#,
            r#"data: {"type":"content_block_stop","index":2}"#,
            r#"event: ping"#,
            r#"data: {"type":"ping"}"#,
            r#"event: message_delta"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"event: message_stop"#,
            r#"data: {"type":"message_stop"}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","delta":{"text":"after"}}"#,
        ],
        r#"terminal=[false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, true, true, true] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"MessageStop","error_code":null,"incomplete_reason":null,"event_count":10,"generated_text":true,"generated_tool_call":true,"generated_thinking":true,"output_bytes":9,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"content_block_delta":3,"content_block_start":2,"content_block_stop":1,"message_delta":1,"message_start":1,"message_stop":1,"ping":1},"termination":"Completed"}"#,
    ),
    (
        "overloaded_error_event",
        &[
            r#"event: error"#,
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#,
        ],
        r#"terminal=[false, true] failure=true hinted=true first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"Overloaded","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"error":1},"termination":"Completed"}"#,
    ),
    (
        "rate_limit_error_event",
        &[
            r#"event: error"#,
            r#"data: {"type":"error","error":{"type":"rate_limit_error"}}"#,
        ],
        r#"terminal=[false, true] failure=true hinted=true first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"RateLimitExceeded","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"error":1},"termination":"Completed"}"#,
    ),
    (
        "malformed_error_event",
        &[r#"event: error"#, r#"data: {SECRET"#],
        r#"terminal=[false, true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":1,"unknown_events":0,"termination":"Completed"}"#,
    ),
    (
        "malformed_message_stop",
        &[r#"event: message_stop"#, r#"data: {SECRET"#],
        r#"terminal=[false, true] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"MessageStop","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":1,"unknown_events":0,"termination":"Completed"}"#,
    ),
    (
        "error_shape_under_other_event",
        &[
            r#"event: ignored"#,
            r#"data: {"type":"error","error":{"type":"overloaded_error"}}"#,
        ],
        r#"terminal=[false, false] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":"Overloaded","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":1,"event_types":{"(unknown)":1},"termination":"Dropped"}"#,
    ),
    (
        "data_without_event_line",
        &[
            r#"data: {"type":"content_block_delta","delta":{"text":"x"}}"#,
            r#"data: {"error":{"type":"server_error"}}"#,
        ],
        r#"terminal=[false, false] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":"ServerError","incomplete_reason":null,"event_count":2,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":1,"oversized_lines":0,"parse_errors":0,"unknown_events":2,"event_types":{"(unknown)":2},"termination":"Dropped"}"#,
    ),
    (
        "done_marker_is_not_terminal",
        &[
            r#"event: message_delta"#,
            r#"data: [DONE]"#,
            r#"data: {"type":"message_delta"}"#,
        ],
        r#"terminal=[false, false, false] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":2,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":1,"unknown_events":0,"event_types":{"message_delta":1},"termination":"Dropped"}"#,
    ),
    (
        "unknown_event_name",
        &[
            r#"event: brand_new"#,
            r#"data: {"type":"brand_new"}"#,
            r#"event: message_start"#,
            r#"data: {"type":"response.completed"}"#,
        ],
        r#"terminal=[false, false, false, false] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":2,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":1,"event_types":{"(unknown)":1,"message_start":1},"termination":"Dropped"}"#,
    ),
];

#[test]
fn the_observer_reads_anthropic_streams_as_it_did() {
    for (name, lines, expected) in CASES {
        assert_eq!(observation(profile(), lines), *expected, "{name}");
    }
}

/// #2236, #2249 review: which events end an attempt and which fail it, on
/// this wire's own vocabulary (moved from `transport/` with the dialects).
#[test]
fn protocol_dispatch_matches_this_wires_terminal_vocabulary() {
    for (event, data, terminal, failure) in [
        (
            "ignored",
            r#"{"type":"error","error":{"type":"overloaded_error"}}"#,
            false,
            false,
        ),
        ("message_stop", "{}", true, false),
        (
            "error",
            r#"{"error":{"type":"overloaded_error"}}"#,
            true,
            true,
        ),
    ] {
        let lines = [format!("event: {event}"), format!("data: {data}")];
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let seen = observed(profile(), &lines, None);
        assert_eq!(
            (seen.terminal[1], seen.failure),
            (terminal, failure),
            "event={event} data={data}"
        );
    }
}

#[test]
fn malformed_anthropic_terminal_retains_event_without_payload_content() {
    for (event, terminal) in [
        ("error", TerminalEvent::Error),
        ("message_stop", TerminalEvent::MessageStop),
    ] {
        let named = format!("event: {event}");
        let seen = observed(profile(), &[&named, "data: {SECRET"], None);
        let d = &seen.diagnostics;
        assert_eq!(d.terminal_event, Some(terminal));
        assert_eq!(d.termination, Termination::Completed);
        assert_eq!(d.parse_errors, 1);
        assert!(!serde_json::to_string(d).unwrap().contains("SECRET"));
    }
}
