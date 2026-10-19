//! #2597: what the attempt observer makes of the Responses (Codex) wire, pinned
//! before attempt observation moved out of `transport/` into per-vendor
//! strategies: every case must read exactly as it did, but for the one row
//! marked a declared change (#2597, M1: each wire knows only its own events).
use crate::domain::inference::value_objects::attempt_diagnostics::Termination;
use crate::infrastructure::providers::transport::attempt_profile::{Profile, Surface};
use crate::infrastructure::providers::transport::attempt_transport::{observation, observed};

fn profile() -> Profile {
    Profile::new(
        &super::codex_wire::CODEX_WIRE,
        Surface::Incremental,
        Default::default(),
    )
}

/// Each case: its name, the SSE lines, and the observation they make.
const CASES: &[(&str, &[&str], &str)] = &[
    (
        "tool_call_text_reasoning_and_completed",
        &[
            r#"data: {"type":"response.created","response":{}}"#,
            r#"data: {"type":"response.in_progress"}"#,
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"plan"}"#,
            r#"data: {"type":"response.reasoning.summary_text.delta","delta":"dotted"}"#,
            r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}"#,
            r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\""}"#,
            r#"data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
            r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call"}}"#,
            r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#,
            r#"data: {"type":"response.refusal.delta","delta":"no"}"#,
            r#"data: {"type":"response.never_heard_of"}"#,
            r#"data: {"type":"response.completed","response":{"status":"completed"}}"#,
            r#"data: {"type":"response.output_text.delta","delta":"after"}"#,
        ],
        r#"terminal=[false, false, false, false, false, false, false, false, false, false, false, true, true] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"ResponseCompleted","error_code":null,"incomplete_reason":null,"event_count":12,"generated_text":true,"generated_tool_call":true,"generated_thinking":true,"output_bytes":24,"oversized_lines":0,"parse_errors":0,"unknown_events":1,"event_types":{"(unknown)":1,"response.completed":1,"response.created":1,"response.function_call_arguments.delta":1,"response.function_call_arguments.done":1,"response.in_progress":1,"response.output_item.added":1,"response.output_item.done":1,"response.output_text.delta":1,"response.reasoning.summary_text.delta":1,"response.reasoning_summary_text.delta":1,"response.refusal.delta":1},"termination":"Completed"}"#,
    ),
    (
        "response_failed",
        &[r#"data: {"type":"response.failed","response":{"error":{"code":"server_error"}}}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"ResponseFailed","error_code":"ServerError","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"response.failed":1},"termination":"Completed"}"#,
    ),
    (
        // #2593: a reply cut off at its output limit completes.
        "response_incomplete",
        &[
            r#"data: {"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#,
        ],
        r#"terminal=[true] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"ResponseIncomplete","error_code":null,"incomplete_reason":"MaxOutputTokens","event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"response.incomplete":1},"termination":"Completed"}"#,
    ),
    (
        "typed_error_event_rate_limit",
        &[
            r#"data: {"type":"error","error":{"type":"rate_limit_exceeded","code":"rate_limit_exceeded"}}"#,
        ],
        r#"terminal=[true] failure=true hinted=true first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"RateLimitExceeded","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"error":1},"termination":"Completed"}"#,
    ),
    (
        "untyped_error_chunk",
        &[r#"data: {"error":{"type":"server_error"}}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"ServerError","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"(unknown)":1},"termination":"Completed"}"#,
    ),
    (
        "null_typed_error_chunk",
        &[r#"data: {"type":null,"error":"boom"}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"(unknown)":1},"termination":"Completed"}"#,
    ),
    (
        "typed_event_with_error_field",
        &[
            r#"data: {"type":"response.output_text.delta","delta":"x","error":{"type":"server_error"}}"#,
        ],
        r#"terminal=[false] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":"ServerError","incomplete_reason":null,"event_count":1,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":1,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"response.output_text.delta":1},"termination":"Dropped"}"#,
    ),
    (
        "done_marker",
        &[
            r#"data: {"type":"response.output_text.delta","delta":"x"}"#,
            r#"data: [DONE]"#,
            r#"data: {"type":"response.completed"}"#,
        ],
        r#"terminal=[false, true, true] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Done","error_code":null,"incomplete_reason":null,"event_count":2,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":1,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"[DONE]":1,"response.output_text.delta":1},"termination":"Completed"}"#,
    ),
    (
        "other_wires_vocabulary",
        &[
            r#"data: {"type":"ping"}"#,
            r#"data: {"type":"message_start"}"#,
            r#"data: {"choices":[{"delta":{"content":"c"}}]}"#,
            r#"data: {"type":"message_stop"}"#,
        ],
        // Declared change (#2597, M1): the Responses wire knows only its own
        // events, so another wire's events are unknown, generate nothing and
        // end nothing.
        r#"terminal=[false, false, false, false] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":4,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":4,"event_types":{"(unknown)":4},"termination":"Dropped"}"#,
    ),
    (
        "malformed_and_event_lines",
        &[
            r#"event: response.created"#,
            r#"data: {broken"#,
            r#"data: {"type":"response.output_text.delta","delta":"ok"}"#,
        ],
        r#"terminal=[false, false, false] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":2,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":2,"oversized_lines":0,"parse_errors":1,"unknown_events":0,"event_types":{"response.output_text.delta":1},"termination":"Dropped"}"#,
    ),
];

#[test]
fn the_observer_reads_codex_streams_as_it_did() {
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
            "",
            r#"{"type":"extension","error":{"code":"rate_limit_exceeded"}}"#,
            false,
            false,
        ),
        (
            "",
            r#"{"type":"response.completed","response":{}}"#,
            true,
            false,
        ),
        (
            "",
            r#"{"type":"error","code":"rate_limit_exceeded"}"#,
            true,
            true,
        ),
        (
            "",
            r#"{"type":"response.failed","response":{"error":{}}}"#,
            true,
            true,
        ),
        (
            "",
            r#"{"type":"response.incomplete","response":{}}"#,
            true,
            true,
        ),
        // #2593: a reply cut off at its output limit completes; one
        // incomplete for another reason still fails.
        (
            "",
            r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#,
            true,
            false,
        ),
        (
            "",
            r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}"#,
            true,
            true,
        ),
        ("", r#"{"error":"model crashed"}"#, true, true),
        ("", r#"{"error":{"message":"model crashed"}}"#, true, true),
        (
            "",
            r#"{"type":null,"error":{"message":"model crashed"}}"#,
            true,
            true,
        ),
        ("", r#"{"error":""}"#, false, false),
        ("", r#"{"error":{}}"#, true, true),
        (
            "",
            r#"{"type":"response.created","error":"x"}"#,
            false,
            false,
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
fn supported_dotted_reasoning_remains_visible_on_read_failure() {
    let seen = observed(
        profile(),
        &[r#"data: {"type":"response.reasoning.summary_text.delta","delta":"SECRET"}"#],
        Some(Termination::ReadError),
    );
    let d = &seen.diagnostics;
    assert!(d.generated_thinking);
    assert_eq!(d.unknown_events, 0);
    assert_eq!(d.termination, Termination::ReadError);
    assert!(!serde_json::to_string(d).unwrap().contains("SECRET"));
}

/// #2158: a recorded Codex tool-call stream counts no unknown events; an
/// event nobody knows still counts, before the terminal event, after which
/// nothing is counted.
#[test]
fn a_codex_tool_call_stream_has_no_unknown_events() {
    let stream = [
        r#"data: {"type":"response.created","response":{}}"#,
        r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}"#,
        r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\""}"#,
        r#"data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"a\"}"}"#,
        r#"data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
        r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call"}}"#,
    ];
    assert_eq!(
        observed(profile(), &stream, None)
            .diagnostics
            .unknown_events,
        0
    );
    let mut ended = stream.to_vec();
    ended.push(r#"data: {"type":"response.never_heard_of"}"#);
    ended.push(r#"data: {"type":"response.completed","response":{"status":"completed"}}"#);
    ended.push(r#"data: {"type":"response.never_heard_of"}"#);
    assert_eq!(
        observed(profile(), &ended, None).diagnostics.unknown_events,
        1
    );
}

/// #2155 review: another wire's numeric throttle code does not throttle a
/// Responses attempt; its typed fields alone decide.
#[test]
fn a_numeric_throttle_code_does_not_throttle_a_responses_attempt() {
    let seen = observed(
        profile(),
        &[r#"data: {"type":"error","error":{"code":429}}"#],
        None,
    );
    assert!(!seen.hinted);
    assert!(seen.failure, "an error event always fails the attempt");
}
