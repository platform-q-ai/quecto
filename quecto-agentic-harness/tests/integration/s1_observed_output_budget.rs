//! Actual payload oracle: complete text/tool lifecycles, not observer flags.
use super::*;
use quecto::domain::inference::value_objects::provider::StreamEvent;
use quecto::domain::message::LlmResponse;
use serde_json::{Value, json};

fn wire(leaf: Leaf, event: &str, mut value: Value, metadata: bool) -> String {
    if metadata {
        let error = json!({"type":"rate_limit_error","code":"rate_limit_exceeded","message":"CANARY_METADATA"});
        // OpenAI's root error is an actual explicit error chunk, not metadata.
        value["response"] = json!({"error":error});
        if matches!(leaf, Leaf::Anthropic | Leaf::Responses | Leaf::OAuth) {
            value["error"] = error;
        }
    }
    format!(
        "{}data: {value}\n\n",
        if matches!(leaf, Leaf::Anthropic) {
            format!("event: {event}\n")
        } else {
            String::new()
        }
    )
}
fn terminal(leaf: Leaf) -> &'static str {
    match leaf {
        Leaf::OpenAi => "data: [DONE]\n\n",
        _ => leaf.terminal(),
    }
}
fn text_body(leaf: Leaf, text: &str, metadata: bool) -> String {
    let mut body = String::new();
    match leaf {
        Leaf::OpenAi => body.push_str(&wire(
            leaf,
            "",
            json!({"choices":[{"index":0,"delta":{"content":text}}]}),
            metadata,
        )),
        Leaf::Anthropic => {
            body.push_str(&wire(leaf, "message_start", json!({"type":"message_start","message":{"id":"msg","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}), false));
            body.push_str(&wire(leaf, "content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}), metadata));
            body.push_str(&wire(leaf, "content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}), metadata));
            body.push_str(&wire(
                leaf,
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
                false,
            ));
        }
        _ => body.push_str(&wire(
            leaf,
            "",
            json!({"type":"response.output_text.delta","delta":text}),
            metadata,
        )),
    }
    body.push_str(terminal(leaf));
    body
}
fn tool_body(leaf: Leaf, arguments: &str, metadata: bool) -> String {
    let mut body = String::new();
    match leaf {
        Leaf::OpenAi => {
            body.push_str(&wire(leaf, "", json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":"fixture","arguments":""}}]}}]}), metadata));
            body.push_str(&wire(leaf, "", json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":arguments}}]},"finish_reason":"tool_calls"}]}), metadata));
        }
        Leaf::Anthropic => {
            body.push_str(&wire(leaf, "message_start", json!({"type":"message_start","message":{"id":"msg","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}}), false));
            body.push_str(&wire(leaf, "content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":"fixture","input":{}}}), metadata));
            body.push_str(&wire(leaf, "content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":arguments}}), metadata));
            body.push_str(&wire(
                leaf,
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
                false,
            ));
            body.push_str(&wire(leaf, "message_delta", json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}), false));
        }
        _ => {
            body.push_str(&wire(leaf, "", json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"item","call_id":"call","name":"fixture","arguments":""}}), metadata));
            body.push_str(&wire(leaf, "", json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"item","delta":arguments}), metadata));
            body.push_str(&wire(leaf, "", json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"item","arguments":arguments}), false));
            body.push_str(&wire(leaf, "", json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"item","call_id":"call","name":"fixture","arguments":arguments}}), false));
        }
    }
    body.push_str(terminal(leaf));
    body
}
async fn payload(
    leaf: Leaf,
    surface: Surface,
    admitted: bool,
    body: String,
) -> (LlmResponse, Vec<StreamEvent>, Arc<RequestTrace>, Snapshot) {
    let server = Server::start(WireReply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body,
        stalled: false,
    })
    .await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let mut req = request();
    req.trace = Some(trace.clone());
    let mut events = Vec::new();
    let response = match surface {
        Surface::Chat => bounded(provider.chat(req)).await.unwrap(),
        Surface::Assembled => bounded(provider.chat_stream(req)).await.unwrap(),
        Surface::Incremental => {
            let mut rx = provider.chat_stream_incremental(req).await;
            let mut done = None;
            while let Some(event) = bounded(rx.recv()).await {
                if let StreamEvent::Done(response) = &event {
                    assert!(done.is_none(), "one Done contract");
                    done = Some(response.clone());
                }
                assert!(
                    matches!(
                        event,
                        StreamEvent::TextDelta(_)
                            | StreamEvent::ThinkingDelta(_)
                            | StreamEvent::ToolCallStart { .. }
                            | StreamEvent::ToolCallDelta(_)
                            | StreamEvent::ToolCallEnd { .. }
                            | StreamEvent::Done(_)
                    ),
                    "{event:?}"
                );
                events.push(event);
            }
            done.expect("actual Done response")
        }
    };
    (response, events, trace, gate.snapshot())
}
fn neutral(trace: &RequestTrace, state: &Snapshot, admitted: bool) {
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].error_code, None);
    assert_eq!(state.grants, usize::from(admitted));
    assert_eq!(state.abandoned, 0);
    assert!(state.receipts.is_empty());
    assert!(!state.blocked);
    assert_eq!(
        state.finishes,
        if admitted {
            vec![Feedback::Success]
        } else {
            vec![]
        }
    );
    assert!(!format!("{attempts:?}").contains("CANARY"));
}
#[tokio::test]
async fn s1_observed_error_budget_large_output_and_terminal() {
    for (leaf, surface, admitted) in cells() {
        for size in [8, 6000] {
            for metadata in [false, true] {
                let text = "t".repeat(size);
                let body = format!(
                    "{}{}",
                    text_body(leaf, &text, metadata),
                    envelope(leaf, 4096, false)
                );
                let (response, events, trace, state) = payload(leaf, surface, admitted, body).await;
                assert_eq!(
                    response.content.as_deref(),
                    Some(text.as_str()),
                    "{leaf:?}/{surface:?}/{admitted}/{size}/{metadata}"
                );
                if matches!(surface, Surface::Incremental) {
                    let deltas: String = events
                        .iter()
                        .filter_map(|event| match event {
                            StreamEvent::TextDelta(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(deltas, text);
                }
                neutral(&trace, &state, admitted);
                assert!(trace.attempt_diagnostics()[0].generated_text);
                assert!(trace.attempt_diagnostics()[0].output_bytes >= size as u64);
            }
        }
    }
}
#[tokio::test]
async fn s1_observed_error_budget_large_tool() {
    for (leaf, surface, admitted) in cells() {
        for size in [32, 6000] {
            for metadata in [false, true] {
                let arguments = json!({"arg":"a".repeat(size - 10)}).to_string();
                assert_eq!(arguments.len(), size);
                let (response, events, trace, state) = payload(
                    leaf,
                    surface,
                    admitted,
                    tool_body(leaf, &arguments, metadata),
                )
                .await;
                assert_eq!(
                    response.tool_calls.len(),
                    1,
                    "{leaf:?}/{surface:?}/{admitted}/{size}/{metadata}"
                );
                let tool = &response.tool_calls[0];
                assert_eq!(tool.id, "call");
                assert_eq!(tool.name, "fixture");
                assert_eq!(tool.arguments, arguments);
                // OpenAI/Responses expose tools in Done only; Anthropic also
                // promises incremental Start/Delta/End. Assert each real contract.
                if matches!((leaf, surface), (Leaf::Anthropic, Surface::Incremental)) {
                    let deltas: String = events
                        .iter()
                        .filter_map(|event| match event {
                            StreamEvent::ToolCallDelta(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(deltas, arguments);
                    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolCallStart { id, name } if id == "call" && name == "fixture")));
                    assert!(events.iter().any(|event| matches!(event, StreamEvent::ToolCallEnd { id, name, arguments: actual } if id == "call" && name == "fixture" && actual == &arguments)));
                }
                neutral(&trace, &state, admitted);
                assert!(trace.attempt_diagnostics()[0].generated_tool_call);
                assert!(trace.attempt_diagnostics()[0].output_bytes >= size as u64);
            }
        }
    }
}
