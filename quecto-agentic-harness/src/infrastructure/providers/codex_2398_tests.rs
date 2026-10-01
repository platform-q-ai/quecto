//! #2398: each request records, on its trace, where the input it sends
//! first differs from its session's previous request.
use super::*;
use crate::domain::message::{Message, ToolCall};
use crate::domain::request_observation::{InputItemKind, InputPrefix, RequestTrace};
use std::sync::Arc;

fn provider() -> CodexProvider {
    CodexProvider::with_client(
        "sk-test".into(),
        "acct".into(),
        Some("http://127.0.0.1:9".into()),
        reqwest::Client::new(),
    )
}

/// What `provider` records for one request of `messages` in `session`.
fn observe(provider: &CodexProvider, messages: &[Message], session: &str) -> Option<InputPrefix> {
    let trace = Arc::new(RequestTrace::default());
    send(provider, messages, session, &trace);
    trace.input_prefix()
}

fn send(provider: &CodexProvider, messages: &[Message], session: &str, trace: &Arc<RequestTrace>) {
    let request = ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        messages,
        tools: &[],
        model: "gpt-6-sol",
        max_tokens: 100,
        temperature: 0.0,
        session_id: Some(session),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let _ = provider.prepare(&request);
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }
}

#[test]
fn a_tool_loop_records_append_only_requests() {
    let provider = provider();
    let mut messages = vec![Message::system("sys"), Message::user("list the files")];
    let first = observe(&provider, &messages, "s").expect("observed");
    assert_eq!(first.input_items, 1);
    assert_eq!(first.first_changed_item, None);
    assert_eq!(first.prefix_tokens_estimate, 0);
    messages.push(Message::assistant("", vec![call("c1")]));
    messages.push(Message::tool("c1", "a.rs b.rs"));
    let second = observe(&provider, &messages, "s").expect("observed");
    assert_eq!(second.input_items, 3);
    assert_eq!(second.first_changed_item, None);
    assert_eq!(second.first_changed_kind, None);
    assert!(second.prefix_tokens_estimate > 0, "{second:?}");
}

#[test]
fn an_edited_tool_output_records_its_index_and_kind() {
    let provider = provider();
    let mut messages = vec![
        Message::system("sys"),
        Message::user("go"),
        Message::assistant("", vec![call("c1")]),
        Message::tool("c1", "a very long tool output"),
    ];
    observe(&provider, &messages, "s");
    messages[3] = Message::tool("c1", "[pruned]");
    messages.push(Message::user("next"));
    let observed = observe(&provider, &messages, "s").expect("observed");
    assert_eq!(observed.input_items, 4);
    assert_eq!(observed.first_changed_item, Some(2));
    assert_eq!(
        observed.first_changed_kind,
        Some(InputItemKind::FunctionCallOutput)
    );
    assert!(observed.prefix_tokens_estimate > 0, "{observed:?}");
}

#[test]
fn a_removed_message_records_where_the_inputs_diverge() {
    let provider = provider();
    let before = [
        Message::system("sys"),
        Message::user("1"),
        Message::user("2"),
        Message::user("3"),
    ];
    observe(&provider, &before, "s");
    let after = [
        Message::system("sys"),
        Message::user("1"),
        Message::user("3"),
    ];
    let observed = observe(&provider, &after, "s").expect("observed");
    assert_eq!(observed.first_changed_item, Some(1));
    assert_eq!(observed.first_changed_kind, Some(InputItemKind::User));
}

/// A retry sends the same input again under the same trace: the record of
/// its first send stays.
#[test]
fn a_resent_request_keeps_the_record_of_its_first_send() {
    let provider = provider();
    observe(
        &provider,
        &[Message::system("sys"), Message::user("a")],
        "s",
    );
    let messages = [Message::system("sys"), Message::user("b")];
    let trace = Arc::new(RequestTrace::default());
    send(&provider, &messages, "s", &trace);
    send(&provider, &messages, "s", &trace);
    let observed = trace.input_prefix().expect("observed");
    assert_eq!(observed.first_changed_item, Some(0));
    assert_eq!(observed.first_changed_kind, Some(InputItemKind::User));
}

/// The record carries no content.
#[test]
fn the_record_carries_no_content() {
    const SECRET: &str = "sk-proj-QX7hunter2SECRETtoken9d1f";
    let provider = provider();
    observe(
        &provider,
        &[Message::system("sys"), Message::user(SECRET)],
        "s",
    );
    let observed = observe(
        &provider,
        &[
            Message::system("sys"),
            Message::user(format!("{SECRET}?")),
            Message::assistant(SECRET, vec![call("c1")]),
            Message::tool("c1", SECRET),
        ],
        "s",
    )
    .expect("observed");
    let text = format!("{observed:?} {}", serde_json::to_string(&observed).unwrap());
    for fragment in ["sk-proj", "hunter2", "SECRET", "9d1f"] {
        assert!(!text.contains(fragment), "{fragment} in {text}");
    }
}
