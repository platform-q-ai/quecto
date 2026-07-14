use super::*;
use crate::domain::message::Message;
use crate::interface::cli::uds_session::HISTORY_PAGE_SIZE;

fn message_contents(data: &serde_json::Value) -> Vec<String> {
    data["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|m| m["content"].as_str().expect("content string").to_string())
        .collect()
}

fn snapshot_for(size: usize) -> serde_json::Value {
    let messages: Vec<Message> = (0..size)
        .map(|i| Message::user(format!("msg-{i:03}")))
        .collect();
    let line = build_get_messages_line(&messages);
    let event: serde_json::Value = serde_json::from_str(line.trim()).expect("snapshot event json");
    event["data"].clone()
}

#[test]
fn busy_connect_snapshot_is_newest_page_with_reachable_older_cursor() {
    let data = snapshot_for(HISTORY_PAGE_SIZE * 2);
    let contents = message_contents(&data);

    assert_eq!(contents.len(), HISTORY_PAGE_SIZE);
    assert_eq!(contents.last().map(String::as_str), Some("msg-127"));
    assert_eq!(contents.first().map(String::as_str), Some("msg-064"));
    assert_eq!(data["snapshot"], true);
    assert_eq!(data["hasMoreBefore"], true);
    assert!(data["before"].as_str().is_some());
    assert_ne!(data["trimmed"], true);
}

#[test]
fn busy_connect_snapshot_exact_page_has_no_older_cursor() {
    let data = snapshot_for(HISTORY_PAGE_SIZE);

    assert_eq!(message_contents(&data).len(), HISTORY_PAGE_SIZE);
    assert_eq!(data["hasMoreBefore"], false);
    assert_eq!(data["before"], serde_json::Value::Null);
    assert_ne!(data["trimmed"], true);
}

#[test]
fn busy_connect_snapshot_just_over_page_has_reachable_older_cursor() {
    let data = snapshot_for(HISTORY_PAGE_SIZE + 1);
    let contents = message_contents(&data);

    assert_eq!(contents.len(), HISTORY_PAGE_SIZE);
    assert_eq!(contents.first().map(String::as_str), Some("msg-001"));
    assert_eq!(contents.last().map(String::as_str), Some("msg-064"));
    assert_eq!(data["hasMoreBefore"], true);
    assert!(data["before"].as_str().is_some());
    assert_ne!(data["trimmed"], true);
}

#[test]
fn busy_connect_snapshot_below_page_size_has_no_older_cursor() {
    let data = snapshot_for(3);

    assert_eq!(message_contents(&data), ["msg-000", "msg-001", "msg-002"]);
    assert_eq!(data["hasMoreBefore"], false);
    assert_eq!(data["before"], serde_json::Value::Null);
    assert_ne!(data["trimmed"], true);
}
