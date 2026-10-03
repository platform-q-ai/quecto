// #2403: how a watermark cut's stub goes on the wire. The stub is a user
// message right after the pinned head (the brief, and the latest prompt
// when the turn is still running), so it follows a user message, and a
// tail opening on a prompt follows it. Each is sent as its own `user`
// entry, verbatim and in order: the Messages API combines consecutive
// user turns into one turn, so no role is changed and nothing is merged
// here, and the head's entries stay byte-identical across the cut.

use super::*;
use crate::domain::conversation::watermark_cut::archive_stub;
use crate::domain::message::ToolCall;
use crate::domain::turn_origin::prompt;

fn call(id: &str) -> Message {
    Message::assistant(
        "",
        vec![ToolCall {
            id: id.to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
        }],
    )
}

fn build(messages: &[Message]) -> Vec<serde_json::Value> {
    AnthropicProvider::build_messages(messages, false).1
}

#[test]
fn the_stub_is_its_own_user_entry_between_the_head_and_the_tail() {
    let system = Message::system("SYSTEM");
    let brief = prompt("the brief".to_string());
    let latest = prompt("the latest prompt".to_string());
    let before = vec![
        system.clone(),
        brief.clone(),
        call("c1"),
        Message::tool("c1".to_string(), "old result".to_string()),
        latest.clone(),
        call("c2"),
        Message::tool("c2".to_string(), "new result".to_string()),
    ];
    let stub = archive_stub(2, Some("archive"));
    let after = vec![
        system,
        brief,
        latest,
        stub.clone(),
        call("c2"),
        Message::tool("c2".to_string(), "new result".to_string()),
    ];
    let (before, after) = (build(&before), build(&after));
    assert_eq!(after[0], before[0], "the brief's entry is byte-identical");
    let roles: Vec<&str> = after.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["user", "user", "user", "assistant", "user"]);
    assert_eq!(after[1]["content"], "the latest prompt");
    assert_eq!(
        after[2]["content"],
        serde_json::Value::String(stub.content.trim().to_string()),
        "the stub is sent verbatim, as its own entry"
    );
    assert!(
        after[2]["content"]
            .as_str()
            .is_some_and(|text| text.contains(r#"recall("archive")"#))
    );
}

#[test]
fn a_tail_opening_on_a_prompt_follows_the_stub_as_its_own_entry() {
    let stub = archive_stub(5, Some("archive:2"));
    let messages = vec![
        Message::system("SYSTEM"),
        prompt("the brief".to_string()),
        stub.clone(),
        prompt("the next prompt".to_string()),
        Message::assistant("an answer", vec![]),
    ];
    let wire = build(&messages);
    let users: Vec<&str> = wire[..3]
        .iter()
        .map(|m| m["content"].as_str().unwrap_or("<blocks>"))
        .collect();
    assert_eq!(users, ["the brief", stub.content.trim(), "the next prompt"]);
    assert_eq!(wire[3]["role"], "assistant");
}
