use super::{HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES, HISTORY_PAGE_JSON_BUDGET, messages_page_json};
use crate::domain::message::{Message, ToolCall};

fn message_content<'a>(message: &'a serde_json::Value, field: &str) -> &'a str {
    message
        .get(field)
        .and_then(|value| value.as_str())
        .expect("history message should include requested string field")
}

fn page_messages(page: &serde_json::Value) -> &[serde_json::Value] {
    page.get("messages")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .expect("history page should include messages array")
}

#[test]
fn get_messages_summarises_an_oversized_message_and_keeps_the_page_bounded() {
    let oversized_body = "oversized-output".repeat(HISTORY_PAGE_JSON_BUDGET / 8);
    let messages = vec![
        Message::user("omitted oldest reachable message"),
        Message::user("included older message"),
        Message::assistant(oversized_body.clone(), vec![]),
    ];

    let page = messages_page_json(&messages, 2, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let page_messages = page_messages(&page);
    let oversized = page_messages
        .iter()
        .find(|message| message.get("role").and_then(|value| value.as_str()) == Some("assistant"))
        .expect("oversized assistant message should remain present as a summary");

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "get_messages page should stay within the history byte budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(
        oversized.get("collapsed").and_then(|value| value.as_bool()),
        Some(true),
        "oversized history message should be marked as a summary: {oversized}"
    );
    assert_eq!(
        oversized.get("truncated").and_then(|value| value.as_bool()),
        Some(true),
        "oversized history message should advertise truncation: {oversized}"
    );
    assert_eq!(
        oversized
            .get("contentLength")
            .and_then(|value| value.as_u64()),
        Some(oversized_body.len() as u64),
        "summary should carry the byte length needed for ranged get_message recovery: {oversized}"
    );
    assert!(
        !message_content(oversized, "content").is_empty()
            && message_content(oversized, "content").len() < oversized_body.len(),
        "summary should include only a bounded preview, not the full oversized body"
    );
    assert!(
        oversized
            .get("id")
            .and_then(|value| value.as_str())
            .is_some(),
        "summary should carry a stable message id for get_message recovery: {oversized}"
    );
    assert_eq!(
        page.get("hasMoreBefore").and_then(|value| value.as_bool()),
        Some(true),
        "omitted older history should remain reachable through the before cursor: {page}"
    );
    assert!(
        page.get("before")
            .and_then(|value| value.as_str())
            .is_some(),
        "bounded page with omitted older history should carry a before cursor: {page}"
    );
}

#[test]
fn history_summary_boundary_is_pinned_on_both_sides() {
    let complete_body = "a".repeat(HISTORY_PAGE_JSON_BUDGET / 4);
    let summarised_body = "b".repeat(HISTORY_PAGE_JSON_BUDGET);

    let complete_page = messages_page_json(
        &[Message::assistant(complete_body.clone(), vec![])],
        1,
        None,
    );
    let summarised_page = messages_page_json(
        &[Message::assistant(summarised_body.clone(), vec![])],
        1,
        None,
    );
    let complete = &page_messages(&complete_page)[0];
    let summarised = &page_messages(&summarised_page)[0];

    assert_eq!(message_content(complete, "content"), complete_body);
    assert_eq!(
        complete.get("collapsed").and_then(|value| value.as_bool()),
        Some(false),
        "content well inside the page budget should remain complete: {complete}"
    );
    assert_eq!(
        summarised
            .get("collapsed")
            .and_then(|value| value.as_bool()),
        Some(true),
        "content beyond the page budget should be summarised: {summarised}"
    );
    assert_eq!(
        message_content(summarised, "content").len(),
        HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES,
        "summary preview should be capped at the preview boundary"
    );
    assert_eq!(
        summarised
            .get("contentLength")
            .and_then(|value| value.as_u64()),
        Some(summarised_body.len() as u64)
    );
}

#[test]
fn get_messages_summarises_messages_with_oversized_tool_calls() {
    let oversized_arguments =
        "{\"payload\":\"".to_string() + &"t".repeat(HISTORY_PAGE_JSON_BUDGET) + "\"}";
    let arguments_len = oversized_arguments.len();
    let message = Message::assistant(
        "small assistant content",
        vec![ToolCall {
            id: "call-huge".into(),
            name: "huge_tool".into(),
            arguments: oversized_arguments,
        }],
    );

    let page = messages_page_json(&[message], 1, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let summary = &page_messages(&page)[0];

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "tool-call summary should stay within budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(
        summary.get("collapsed").and_then(|value| value.as_bool()),
        Some(true)
    );
    let tool_calls = summary["toolCalls"]
        .as_array()
        .expect("tool-call summaries");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["id"], "call-huge");
    assert_eq!(tool_calls[0]["name"], "huge_tool");
    assert_eq!(tool_calls[0]["arguments"], "");
    assert_eq!(tool_calls[0]["argumentsLength"], arguments_len);
    assert_eq!(tool_calls[0]["truncated"], true);
    assert!(
        summary.get("id").and_then(|value| value.as_str()).is_some(),
        "summary should carry a stable id so get_message can recover full tool calls: {summary}"
    );
}

#[test]
fn get_messages_keeps_small_messages_complete_and_unstubbed() {
    let messages = vec![
        Message::user("first small message"),
        Message::assistant("second small message", vec![]),
    ];

    let page = messages_page_json(&messages, 2, None);
    let page_messages = page_messages(&page);

    assert_eq!(
        page_messages.len(),
        2,
        "small page should keep the requested messages"
    );
    assert_eq!(
        message_content(&page_messages[0], "content"),
        "first small message"
    );
    assert_eq!(
        message_content(&page_messages[1], "content"),
        "second small message"
    );
    assert!(
        page_messages.iter().all(|message| {
            message.get("collapsed").and_then(|value| value.as_bool()) == Some(false)
                && message.get("truncated").is_none()
                && message.get("contentLength").is_none()
        }),
        "small messages should not be represented as recovery summaries: {page}"
    );
}

#[test]
fn collapsed_history_summary_preserves_visible_thinking() {
    use crate::domain::message::{Message, ThinkingBlock};
    let mut msg = Message::assistant("x".repeat(super::HISTORY_PAGE_JSON_BUDGET), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "visible reasoning".into(),
        signature: "private".into(),
    });

    let summary = super::message_to_json_for_history_page(&msg);

    assert_eq!(summary["collapsed"], true);
    assert_eq!(summary["thinking"][0]["kind"], "text");
    assert_eq!(summary["thinking"][0]["text"], "visible reasoning");
    assert!(!serde_json::to_string(&summary).unwrap().contains("private"));
}
