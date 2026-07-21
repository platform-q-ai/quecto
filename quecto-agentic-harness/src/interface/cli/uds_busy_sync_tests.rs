use super::uds_busy_sync::*;
use crate::domain::message::{Message, ToolCall};
use crate::interface::cli::uds_snapshots::{ConversationSnapshotData, LEDGER_MAX_ENTRIES};

#[test]
fn sync_delta_is_chronological_and_exclusive() {
    let mut snap = ConversationSnapshotData::default();
    let first = Message::user("one");
    let second = Message::assistant("two", vec![]);
    let third = Message::assistant("three", vec![]);
    snap.publish(&[first.clone(), second.clone(), third.clone()]);
    let data = snap.sync_json(0, 1);
    assert_eq!(data["rev"], 3);
    assert!(!data["resync"].as_bool().unwrap());
    assert_eq!(data["caughtUp"], true);
    let ids: Vec<_> = data["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec![second.id().to_string(), third.id().to_string()]);
}

#[test]
fn unchanged_republish_does_not_advance_rev() {
    let mut snap = ConversationSnapshotData::default();
    let msg = Message::user("one");
    let first = snap.publish(std::slice::from_ref(&msg));
    let second = snap.publish(std::slice::from_ref(&msg));
    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(first.rev, second.rev);
}

#[test]
fn sync_payload_preserves_tool_fields() {
    let mut snap = ConversationSnapshotData::default();
    let mut first_tool = Message::tool("call-1", "error-result");
    first_tool.tool_name = Some("bash".into());
    first_tool.is_error = true;
    let mut second_tool = Message::tool("call-2", "ok-result");
    second_tool.tool_name = Some("read".into());
    second_tool.is_error = false;
    let assistant = Message::assistant(
        "",
        vec![
            ToolCall {
                id: "call-1".into(),
                name: "bash".into(),
                arguments: "echo hi".into(),
            },
            ToolCall {
                id: "call-2".into(),
                name: "read".into(),
                arguments: "Cargo.toml".into(),
            },
        ],
    );
    snap.publish(&[assistant, first_tool, second_tool]);
    let data = snap.sync_json(0, 0);
    assert_eq!(data["messages"][0]["toolCalls"][0]["id"], "call-1");
    assert_eq!(data["messages"][0]["toolCalls"][0]["name"], "bash");
    assert_eq!(data["messages"][0]["toolCalls"][0]["arguments"], "echo hi");
    assert_eq!(data["messages"][0]["toolCalls"][1]["id"], "call-2");
    assert_eq!(data["messages"][0]["toolCalls"][1]["name"], "read");
    assert_eq!(data["messages"][1]["toolCallId"], "call-1");
    assert_eq!(data["messages"][1]["toolName"], "bash");
    assert_eq!(data["messages"][1]["isError"], true);
    assert_eq!(data["messages"][2]["toolCallId"], "call-2");
    assert_eq!(data["messages"][2]["toolName"], "read");
    assert_eq!(data["messages"][2]["isError"], false);
}

#[test]
fn epoch_bumps_on_clear_and_sync_demands_resync() {
    let mut snap = ConversationSnapshotData::from_messages(vec![Message::user("old")]);
    let epoch = snap.epoch;
    let rev = snap.rev;
    let adv = snap.clear();
    assert_eq!(adv.epoch, epoch + 1);
    assert_eq!(adv.rev, rev);
    let stale = snap.sync_json(epoch, rev);
    assert!(stale["resync"].as_bool().unwrap());
    assert_eq!(stale["epoch"], adv.epoch);
    assert_eq!(stale["rev"], rev);
    let current = snap.sync_json(adv.epoch, adv.rev);
    assert!(!current["resync"].as_bool().unwrap());
}

#[test]
fn parse_sync_requires_epoch_and_since_rev() {
    let parsed = parse(r#"{"type":"sync","id":"s1","epoch":2,"sinceRev":3}"#).unwrap();
    assert_eq!(parsed.request_id.as_deref(), Some("s1"));
    assert_eq!(parsed.epoch, 2);
    assert_eq!(parsed.since_rev, 3);
    assert_eq!(parse(r#"{"type":"sync","epoch":2}"#), None);
    assert_eq!(
        parse(r#"{"type":"get_state","epoch":2,"sinceRev":3}"#),
        None
    );
    assert_eq!(
        parse(r#"{"type":"sync","agent_id":"child","epoch":2,"sinceRev":3}"#),
        None
    );
    assert_eq!(parse(r#"{"type":"sync","epoch":-1,"sinceRev":3}"#), None);
}

#[test]
fn frontier_eviction_resync_boundary_is_exclusive() {
    let mut snap = ConversationSnapshotData::default();
    let messages: Vec<_> = (0..=LEDGER_MAX_ENTRIES)
        .map(|i| Message::user(format!("message-{i}")))
        .collect();
    let old_first_id = messages[0].id().to_string();
    let retained_first_id = messages[1].id().to_string();
    snap.publish(&messages);

    let retained = snap.sync_json(0, 1);
    assert_eq!(retained["resync"], false);
    assert_eq!(retained["messages"][0]["id"], retained_first_id);
    let evicted = snap.sync_json(0, 0);
    assert_eq!(evicted["resync"], true);
    assert!(
        evicted["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["id"].as_str() != Some(&old_first_id))
    );
}

#[test]
fn reset_to_bumps_epoch_once_without_double_counting_republished_messages() {
    let mut snap = ConversationSnapshotData::from_messages(vec![Message::user("old")]);
    let epoch = snap.epoch;
    let rev = snap.rev;
    let replacement = vec![Message::user("new")];
    let advance = snap.reset_to(&replacement);
    assert_eq!(advance.epoch, epoch + 1);
    assert_eq!(advance.rev, rev + 1);
    assert_eq!(snap.epoch, epoch + 1);
    assert_eq!(snap.rev, rev + 1);
}

#[test]
fn collapse_republish_keeps_message_id_without_advancing_rev() {
    let mut snap = ConversationSnapshotData::default();
    let full = Message::assistant("full content", vec![]);
    let id = full.id().to_string();
    let first = snap.publish(std::slice::from_ref(&full));
    let mut stub = full.clone();
    stub.content = "stub".into();
    stub.is_collapsed = true;
    let second = snap.publish(std::slice::from_ref(&stub));
    let data = snap.sync_json(0, first.rev - 1);
    assert!(!second.changed);
    assert_eq!(second.rev, first.rev);
    assert_eq!(data["messages"][0]["id"], id);
    assert_eq!(data["messages"][0]["content"], "stub");
}
