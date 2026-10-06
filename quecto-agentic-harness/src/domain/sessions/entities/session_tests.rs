use super::*;
use crate::domain::message::{Role, ToolCall};

fn make_tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("id-{}", name),
        name: name.to_string(),
        arguments: "{}".to_string(),
    }
}

fn tool_msg(tool_name: &str, content: &str) -> Message {
    let mut m = Message::tool("some-id", content);
    m.tool_name = Some(tool_name.to_string());
    m
}

fn manifest_msg() -> Message {
    let mut m = Message::assistant("[spill manifest]", vec![]);
    m.is_manifest = true;
    m
}

#[test]
fn test_strip_keeps_user_and_plain_assistant() {
    let messages = vec![
        Message::user("hello"),
        Message::assistant("sure thing", vec![]),
    ];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].role, Role::User);
    assert_eq!(filtered[0].content, "hello");
    assert_eq!(filtered[1].role, Role::Assistant);
    assert_eq!(filtered[1].content, "sure thing");
}

#[test]
fn test_strip_drops_manifest() {
    let messages = vec![Message::user("hello"), manifest_msg()];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].role, Role::User);
}

#[test]
fn test_strip_drops_non_recall_tool_results() {
    let messages = vec![
        Message::user("do something"),
        Message::assistant("", vec![make_tool_call("bash")]),
        tool_msg("bash", "bash output"),
    ];
    let filtered = strip_tool_history(&messages);
    // Only the user message survives (assistant has empty content + tool_call → dropped)
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].role, Role::User);
}

#[test]
fn test_strip_keeps_recall_tool_results_and_paired_assistant() {
    let messages = vec![
        Message::user("what did we do?"),
        Message::assistant("(calls recall)", vec![make_tool_call("recall")]),
        tool_msg("recall", "recalled content"),
    ];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 3);
    assert_eq!(filtered[0].content, "what did we do?");
    assert_eq!(filtered[1].content, "(calls recall)");
    assert_eq!(filtered[2].content, "recalled content");
}

#[test]
fn test_strip_preserves_narrative_text_from_mixed_assistant() {
    let messages = vec![
        Message::user("do something"),
        Message::assistant("I will run bash", vec![make_tool_call("bash")]),
        tool_msg("bash", "bash output"),
    ];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].role, Role::User);
    assert_eq!(filtered[1].role, Role::Assistant);
    assert_eq!(filtered[1].content, "I will run bash");
    // tool_calls must be cleared
    assert!(filtered[1].tool_calls.is_empty());
}

#[test]
fn test_strip_drops_pure_dispatch_assistant_with_no_text() {
    let messages = vec![
        Message::user("run it"),
        Message::assistant("", vec![make_tool_call("bash")]),
        tool_msg("bash", "tool result"),
    ];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].role, Role::User);
}

#[test]
fn test_strip_empty_messages() {
    let messages: Vec<Message> = vec![];
    let filtered = strip_tool_history(&messages);
    assert!(filtered.is_empty());
}

#[test]
fn test_strip_no_tool_messages_unchanged() {
    let messages = vec![
        Message::user("first"),
        Message::assistant("second", vec![]),
        Message::user("third"),
    ];
    let filtered = strip_tool_history(&messages);
    assert_eq!(filtered.len(), 3);
}

// --- #311: filter_orphan_tool_pairs domain function ---

#[test]
fn filter_orphan_pairs_matched_calls_preserved() {
    let mut assistant = Message::assistant("", vec![make_tool_call("bash")]);
    assistant.tool_calls[0].id = "call-1".to_string();
    let mut tool_result = Message::tool("call-1", "output");
    tool_result.tool_call_id = Some("call-1".to_string());
    let messages = vec![assistant, tool_result];
    let (valid, diag) = filter_orphan_tool_pairs(&messages);
    assert!(valid.contains("call-1"));
    assert!(!diag.has_orphans());
}

#[test]
fn filter_orphan_pairs_unmatched_call_excluded() {
    let mut assistant = Message::assistant("", vec![make_tool_call("bash")]);
    assistant.tool_calls[0].id = "call-orphan".to_string();
    // No matching tool result
    let messages = vec![assistant];
    let (valid, diag) = filter_orphan_tool_pairs(&messages);
    assert!(!valid.contains("call-orphan"));
    assert!(diag.has_orphans());
    assert!(diag.orphaned_calls.contains(&"call-orphan".to_string()));
}

#[test]
fn filter_orphan_pairs_unmatched_result_excluded() {
    let mut tool_result = Message::tool("ghost-id", "output");
    tool_result.tool_call_id = Some("ghost-id".to_string());
    // No matching assistant call
    let messages = vec![tool_result];
    let (valid, diag) = filter_orphan_tool_pairs(&messages);
    assert!(!valid.contains("ghost-id"));
    assert!(diag.has_orphans());
    assert!(diag.orphaned_results.contains(&"ghost-id".to_string()));
}

#[test]
fn filter_orphan_pairs_empty_messages_returns_empty() {
    let messages: Vec<Message> = vec![];
    let (valid, diag) = filter_orphan_tool_pairs(&messages);
    assert!(valid.is_empty());
    assert!(!diag.has_orphans());
}
