//! #2597: what the attempt observer makes of the OpenAI chat wire, pinned
//! before attempt observation moved out of `transport/` into per-vendor
//! strategies: every case must read exactly as it did.
use crate::domain::inference::value_objects::attempt_diagnostics::{TerminalEvent, Termination};
use crate::infrastructure::providers::transport::attempt_profile::{Profile, Surface};
use crate::infrastructure::providers::transport::attempt_transport::{observation, observed};

fn profile() -> Profile {
    Profile::new(
        &super::openai_wire::OPENAI_WIRE,
        Surface::Incremental,
        Default::default(),
    )
}

/// Each case: its name, the SSE lines, and the observation they make.
const CASES: &[(&str, &[&str], &str)] = &[
    (
        "text_reasoning_tools_and_done",
        &[
            r#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"data: {"choices":[{"delta":{"reasoning":"think"}}]}"#,
            r#"data: {"choices":[{"delta":{"reasoning_content":"more"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Hello"}}]}"#,
            r#"data: {"choices":[{"delta":{"refusal":"no"}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":"{\"p\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"data: [DONE]"#,
            r#"data: {"choices":[{"delta":{"content":"after done"}}]}"#,
        ],
        r#"terminal=[false, false, false, false, false, false, false, true, true] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Done","error_code":null,"incomplete_reason":null,"event_count":8,"generated_text":true,"generated_tool_call":true,"generated_thinking":true,"output_bytes":20,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"[DONE]":1,"chat.delta.content":1,"chat.delta.reasoning":2,"chat.delta.refusal":1,"chat.delta.tool_calls":1,"chat.empty":1,"chat.finish":1},"termination":"Completed"}"#,
    ),
    (
        "typed_rate_limit_error_chunk",
        &[
            r#"data: {"choices":[{"delta":{"content":"partial"}}]}"#,
            r#"data: {"error":{"code":"rate_limit_exceeded","message":"slow down"}}"#,
            r#"data: {"choices":[{"delta":{"content":"ignored"}}]}"#,
        ],
        r#"terminal=[false, true, true] failure=true hinted=true first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"RateLimitExceeded","incomplete_reason":null,"event_count":2,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":7,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.delta.content":1,"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "numeric_429_error_chunk",
        &[r#"data: {"error":{"code":429,"message":"busy"}}"#],
        r#"terminal=[true] failure=true hinted=true first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "numeric_529_error_chunk",
        &[r#"data: {"error":{"code":529}}"#],
        r#"terminal=[true] failure=true hinted=true first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "string_error_chunk",
        &[r#"data: {"error":"model crashed"}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "billing_error_chunk",
        &[r#"data: {"error":{"type":"insufficient_quota"}}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":"InsufficientQuota","incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "empty_error_object",
        &[r#"data: {"error":{}}"#],
        r#"terminal=[true] failure=true hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"Error","error_code":null,"incomplete_reason":null,"event_count":1,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":0,"event_types":{"chat.empty":1},"termination":"Completed"}"#,
    ),
    (
        "non_error_shapes",
        &[
            r#"data: {"error":null}"#,
            r#"data: {"error":""}"#,
            r#"data: {"error":3}"#,
            r#"data: {"choices":[]}"#,
        ],
        r#"terminal=[false, false, false, false] failure=false hinted=false first_token=false
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":4,"generated_text":false,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":3,"event_types":{"chat.empty":4},"termination":"Dropped"}"#,
    ),
    (
        "malformed_then_text",
        &[
            r#"data: {nope"#,
            r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            r#"event: error"#,
            r#"retry: 5"#,
            r#""#,
        ],
        r#"terminal=[false, false, false, false, false] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":null,"error_code":null,"incomplete_reason":null,"event_count":2,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":2,"oversized_lines":0,"parse_errors":1,"unknown_events":0,"event_types":{"chat.delta.content":1},"termination":"Dropped"}"#,
    ),
    (
        "other_wires_vocabulary",
        &[
            r#"data: {"type":"message_start"}"#,
            r#"data: {"type":"response.output_text.delta","delta":"x"}"#,
            r#"data: {"type":"response.completed"}"#,
            r#"data: {"type":"never_heard_of"}"#,
            r#"data: {"delta":{"text":"t"}}"#,
        ],
        r#"terminal=[false, false, false, false, false] failure=false hinted=false first_token=true
{"attempt_number":0,"started_unix_ms":0,"finished_unix_ms":0,"elapsed_ms":0,"wire_status":null,"headers":[],"stop_reason":null,"terminal_event":"ResponseCompleted","error_code":null,"incomplete_reason":null,"event_count":5,"generated_text":true,"generated_tool_call":false,"generated_thinking":false,"output_bytes":0,"oversized_lines":0,"parse_errors":0,"unknown_events":2,"event_types":{"chat.empty":5},"termination":"Completed"}"#,
    ),
];

#[test]
fn the_observer_reads_openai_streams_as_it_did() {
    for (name, lines, expected) in CASES {
        assert_eq!(observation(profile(), lines), *expected, "{name}");
    }
}

/// #2236, #2249 review: which events end an attempt and which fail it, on
/// this wire's own vocabulary (moved from `transport/` with the dialects).
#[test]
fn protocol_dispatch_matches_this_wires_terminal_vocabulary() {
    for (event, data, terminal, failure) in [
        ("", r#"{"type":"response.completed"}"#, false, false),
        (
            "",
            r#"{"choices":[{"finish_reason":"stop"}]}"#,
            false,
            false,
        ),
        (
            "",
            r#"{"type":"error","code":"rate_limit_exceeded"}"#,
            false,
            false,
        ),
        (
            "",
            r#"{"error":{"code":"rate_limit_exceeded"}}"#,
            true,
            true,
        ),
        ("", r#"{"error":"model crashed"}"#, true, true),
        ("", r#"{"error":null}"#, false, false),
        ("", r#"{"error":42}"#, false, false),
        ("", r#"{"error":["x"]}"#, false, false),
        ("", r#"{"error":""}"#, false, false),
        ("", r#"{"error":{}}"#, true, true),
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
fn supported_error_and_reasoning_events_retain_truthful_metadata() {
    let seen = observed(
        profile(),
        &[r#"data: {"error":{"code":"rate_limit_exceeded","message":"SECRET"}}"#],
        None,
    );
    assert_eq!(seen.diagnostics.terminal_event, Some(TerminalEvent::Error));
    assert_eq!(seen.diagnostics.termination, Termination::Completed);
}

/// #2155 review: an OpenAI-compatible error chunk with a numeric 429 or 529
/// (OpenRouter's shape) throttles admission like a typed one; a billing
/// error or a server status does not.
#[test]
fn numeric_throttle_chunks_throttle_admission() {
    for (data, throttled) in [
        (r#"{"error":{"code":429,"message":"m"}}"#, true),
        (r#"{"error":{"code":529}}"#, true),
        (r#"{"error":{"type":"rate_limit_error"}}"#, true),
        (r#"{"error":{"code":500,"type":"rate_limit_error"}}"#, true),
        (
            r#"{"error":{"code":429,"type":"insufficient_quota"}}"#,
            false,
        ),
        (r#"{"error":{"code":529,"type":"billing_error"}}"#, false),
        (r#"{"error":{"code":502}}"#, false),
        (r#"{"error":{"type":"server_error"}}"#, false),
    ] {
        let seen = observed(profile(), &[&format!("data: {data}")], None);
        assert_eq!(seen.hinted, throttled, "{data}");
        assert!(
            seen.failure,
            "an error chunk always fails the attempt: {data}"
        );
    }
}

/// #2236: through admission, a mid-stream error chunk of either shape ends
/// the stream as an error after the partial text, never as a `Done` reply,
/// and the end of file after it cannot turn the partial text into a whole
/// answer.
#[tokio::test]
async fn an_error_chunk_of_either_shape_ends_an_admitted_stream_as_an_error() {
    use crate::domain::inference::value_objects::provider::StreamEvent;
    use crate::infrastructure::providers::transport::attempt_transport::observed_stream;
    let text = r#"data: {"choices":[{"index":0,"delta":{"content":"partial"}}]}"#;
    for (error, status) in [
        (r#"{"error":"model crashed"}"#, "HTTP 502 "),
        (r#"{"error":{"message":"model crashed"}}"#, "HTTP 502 "),
        (r#"{"error":{"type":"invalid_request_error"}}"#, "HTTP 400 "),
    ] {
        let line = format!("data: {error}");
        let (done, events, terminal) = observed_stream(
            profile(),
            super::openai_sse::OpenAiSseHandler::with_model("m"),
            &[text, &line],
        )
        .await;
        assert_eq!(done, [false, true], "{error}");
        assert!(
            matches!(&events[..], [StreamEvent::TextDelta(t), StreamEvent::Error(e)]
                if t == "partial" && e.starts_with(status)),
            "{error}: {events:?}"
        );
        assert_eq!(terminal, Some(TerminalEvent::Error));
    }
}
