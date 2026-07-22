use super::*;
use serde_json::json;

fn delta(messages: Vec<serde_json::Value>, resync: bool) -> SyncDelta {
    SyncDelta {
        epoch: 1,
        rev: 9,
        messages,
        next_rev: None,
        caught_up: true,
        resync,
    }
}

#[test]
fn apply_sync_delta_is_idempotent_by_message_id() {
    let mut t = LedgerTranscript::default();
    let d = delta(
        vec![json!({"id":"u1","role":"user","content":"hello"})],
        false,
    );
    assert_eq!(t.apply_sync_delta(&d).len(), 1);
    let entries = t.apply_sync_delta(&d);
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0], ChatEntry::User { text } if text == "hello"));
}

#[test]
fn apply_sync_delta_preserves_tool_cards() {
    let mut t = LedgerTranscript::default();
    let entries = t.apply_sync_delta(&delta(vec![
            json!({"id":"a1","role":"assistant","content":"done","toolCalls":[{"id":"tc1","name":"bash","arguments":{"command":"echo hi"}}]}),
            json!({"id":"t1","role":"tool","toolCallId":"tc1","toolName":"bash","content":"hi\n","isError":false}),
        ], false));
    assert!(
        matches!(&entries[0], ChatEntry::ToolExecution { tool_call_id, tool_name, result: Some(result), is_error, .. } if tool_call_id == "tc1" && tool_name == "bash" && result == "hi\n" && !is_error)
    );
    assert!(
        matches!(&entries[1], ChatEntry::Assistant { text, streaming: false } if text == "done")
    );
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
    assert!(matches!(&entries[0], ChatEntry::User { text } if text == "new"));
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
    assert!(matches!(&entries[0], ChatEntry::Assistant { text, .. } if text == "complete"));
}

#[test]
fn capability_parsing_accepts_top_level_or_nested_sync_one() {
    assert!(supports_sync(&json!({"sync":1})));
    assert!(supports_sync(&json!({"capabilities":{"sync":1}})));
    assert!(supports_sync(&json!({"sync":2})));
    assert!(!supports_sync(&json!({"sync":0})));
    assert!(!supports_sync(&json!({"sync":"1"})));
    assert!(!supports_sync(&json!({"capabilities":{"sync":0}})));
}

#[test]
fn sync_delta_deserializes_continuation_cursor() {
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
