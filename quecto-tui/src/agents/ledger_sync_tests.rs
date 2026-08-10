use super::*;
use crate::protocol::agent_ledger_payloads::{SyncDelta, supports_sync};
use serde_json::json;

fn message(value: serde_json::Value) -> crate::protocol::agent_ledger_payloads::LedgerMessage {
    serde_json::from_value(value).unwrap()
}

fn delta(messages: Vec<serde_json::Value>, resync: bool) -> SyncDelta {
    SyncDelta {
        epoch: 1,
        rev: 9,
        messages: messages.into_iter().map(message).collect(),
        next_rev: None,
        caught_up: true,
        resync,
    }
}

#[test]
fn repeated_sync_delta_does_not_duplicate_messages() {
    let mut t = LedgerTranscript::default();
    let d = delta(
        vec![json!({"id":"u1","role":"user","content":"hello"})],
        false,
    );
    assert_eq!(t.apply_sync_delta(&d).len(), 1);
    let entries = t.apply_sync_delta(&d);
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0], LedgerEntry::User { text } if text == "hello"));
}

#[test]
fn synced_tool_calls_and_results_render_as_tool_cards() {
    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta(vec![
            json!({"id":"a1","role":"assistant","content":"done","toolCalls":[{"id":"tc1","name":"bash","arguments":{"command":"echo hi"}}]}),
            json!({"id":"t1","role":"tool","toolCallId":"tc1","toolName":"bash","content":"hi\n","isError":false}),
        ], false));
    assert!(
        matches!(&entries[0], LedgerEntry::ToolExecution { tool_call_id, tool_name, result: Some(result), is_error, .. } if tool_call_id == "tc1" && tool_name == "bash" && result == "hi\n" && !is_error)
    );
    assert!(matches!(&entries[1], LedgerEntry::Assistant { text } if text == "done"));
}

#[test]
fn resync_clears_stale_transcript() {
    let mut t = LedgerTranscript::default();
    t.apply_sync_delta(&delta(
        vec![json!({"id":"old","role":"user","content":"old"})],
        false,
    ));
    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"new","role":"user","content":"new"})],
        true,
    ));
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0], LedgerEntry::User { text } if text == "new"));
}

#[test]
fn continuation_upserts_stub_by_id() {
    let mut t = LedgerTranscript::default();
    t.apply_sync_delta(&delta(
        vec![json!({"id":"m1","role":"assistant","content":"partial","collapsed":true})],
        false,
    ));
    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"m1","role":"assistant","content":"complete"})],
        false,
    ));
    assert!(matches!(&entries[0], LedgerEntry::Assistant { text } if text == "complete"));
}

#[test]
fn capability_parsing_accepts_top_level_or_nested_sync_one() {
    assert!(supports_sync(&json!({"sync":1})));
    assert!(supports_sync(&json!({"capabilities":{"sync":1}})));
    assert!(supports_sync(&json!({"sync":2})));
    assert!(!supports_sync(&json!({"sync":0})));
    assert!(!supports_sync(&json!({"sync":"1"})));
    assert!(!supports_sync(&json!({"capabilities":{"sync":0}})));
    assert!(
        supports_sync(&json!({"sync":1,"capabilities":[]})),
        "top-level sync support must survive unrelated malformed capabilities data"
    );
}

#[test]
fn sync_response_can_request_a_follow_up_page() {
    let d: SyncDelta = serde_json::from_value(json!({
        "epoch":7,
        "rev":11,
        "messages":[],
        "nextRev":12,
        "caughtUp":false,
        "resync":false
    }))
    .unwrap();
    assert_eq!(d.epoch, 7);
    assert_eq!(d.rev, 11);
    assert_eq!(d.next_rev, Some(12));
    assert!(!d.caught_up);
    assert!(!d.resync);
}

#[test]
fn one_malformed_message_does_not_discard_the_whole_sync_delta() {
    // The pre-typed implementation stored raw JSON and defaulted bad fields at
    // projection time, so the delta still applied and the revision cursor still
    // advanced. Strict per-message typing must not regress that.
    let delta = serde_json::from_value::<SyncDelta>(json!({
        "epoch": 7,
        "rev": 12,
        "messages": [
            {"id": "bad", "role": "user", "content": 123},
            {"id": 456, "role": "user", "content": "id is not a string"},
            {"id": "badcalls", "role": "assistant", "content": "text",
             "toolCalls": {"not": "an array"}},
            {"id": "baderr", "role": "tool", "toolCallId": "tc9",
             "toolName": "bash", "content": "out", "isError": "true"},
            {"id": "ok", "role": "user", "content": "shown"}
        ],
        "nextRev": null,
        "caughtUp": true,
        "resync": false
    }))
    .expect("a malformed message must not fail the whole delta");
    assert_eq!(delta.rev, 12);

    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta);

    // Malformed text/id fields project as absent, exactly as the raw-JSON
    // projection did; the valid messages still render.
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, LedgerEntry::User { text } if text == "shown")),
        "the valid message must still render: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, LedgerEntry::Assistant { text } if text == "text")),
        "a message with a malformed toolCalls field still renders its content: {entries:?}"
    );
    assert!(
        entries.iter().any(
            |e| matches!(e, LedgerEntry::ToolExecution { tool_call_id, is_error, .. }
                if tool_call_id == "tc9" && !is_error)
        ),
        "a non-bool isError defaults to false rather than failing the delta: {entries:?}"
    );
}

#[test]
fn explicit_null_tool_arguments_render_as_null_not_empty_object() {
    // Missing arguments and an explicit JSON null were distinct before the
    // typed DTOs: only a missing field defaulted to "{}".
    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"a1","role":"assistant","content":"",
                    "toolCalls":[{"id":"tc1","name":"bash","arguments":null}]})],
        false,
    ));
    assert!(
        matches!(&entries[0], LedgerEntry::ToolExecution { args, .. } if args == "null"),
        "explicit null arguments must not collapse to the missing-field default: {entries:?}"
    );

    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"a2","role":"assistant","content":"",
                    "toolCalls":[{"id":"tc2","function":{"name":"bash","arguments":null}}]})],
        false,
    ));
    assert!(
        matches!(&entries[0], LedgerEntry::ToolExecution { args, tool_name, .. }
            if args == "null" && tool_name == "bash"),
        "nested function arguments follow the same rule: {entries:?}"
    );

    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"a3","role":"assistant","content":"",
                    "toolCalls":[{"id":"tc3","name":"bash"}]})],
        false,
    ));
    assert!(
        matches!(&entries[0], LedgerEntry::ToolExecution { args, .. } if args == "{}"),
        "a missing arguments field still defaults to an empty object: {entries:?}"
    );
}

#[test]
fn ledger_1196_transcript_retention_is_bounded_but_can_recover_from_sync() {
    let mut t = LedgerTranscript::default();
    let many = (0..(LEDGER_RETAINED_MESSAGE_CAP + 50))
        .map(|i| json!({"id": format!("m-{i}"), "role":"user", "content": format!("msg-{i}")}))
        .collect();
    t.apply_sync_delta(&delta(many, false));
    assert!(t.retained_message_count() <= LEDGER_RETAINED_MESSAGE_CAP);

    let entries = t.apply_sync_delta(&delta(
        vec![json!({"id":"older-0","role":"user","content":"older recovered"})],
        true,
    ));
    assert!(t.retained_message_count() <= LEDGER_RETAINED_MESSAGE_CAP);
    assert!(
        entries
            .iter()
            .any(|e| matches!(e, LedgerEntry::User { text } if text == "older recovered"))
    );
}
