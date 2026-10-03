use super::*;

#[test]
fn ledger_advanced_is_typed_and_preserves_epoch_rev() {
    let event: AgentEvent = serde_json::from_str(r#"{"type":"ledger_advanced","epoch":4,"rev":9}"#)
        .expect("ledger_advanced parses");

    match event {
        AgentEvent::LedgerAdvanced { epoch, rev } => {
            assert_eq!(epoch, 4);
            assert_eq!(rev, 9);
        }
        other => panic!("ledger_advanced must not collapse to {other:?}"),
    }
}

#[test]
fn future_ledger_events_preserve_payload() {
    for (wire, expected_type) in [
        (
            r#"{"type":"ledger_checkpoint","epoch":4,"rev":10,"future":{"x":1}}"#,
            "ledger_checkpoint",
        ),
        (
            r#"{"type":"ledger_resync_required","epoch":2,"sinceRev":9,"resync":{"reason":"compacted"}}"#,
            "ledger_resync_required",
        ),
    ] {
        let event: AgentEvent = serde_json::from_str(wire).expect("future ledger event parses");
        let json = serde_json::to_value(event).expect("serializes");
        assert_eq!(json["type"], expected_type);
        assert!(json.get("epoch").is_some(), "epoch preserved in {json}");
    }
}

/// #2434: `agent_end` forwards the run's reply text length, 0 when the run
/// recorded no reply, so a WS client can tell there is nothing to rebuild.
#[test]
fn agent_end_forwards_its_content_length() {
    let wire = r#"{"type":"agent_end","messages":[],"messageRefs":["a","b"],"contentLength":0}"#;
    let event: AgentEvent = serde_json::from_str(wire).expect("agent_end parses");
    let json = serde_json::to_value(event).expect("serializes");
    assert_eq!(json["contentLength"], 0, "{json}");
    assert_eq!(json["messageRefs"], serde_json::json!(["a", "b"]));
}

/// An older harness sends none, and none is forwarded.
#[test]
fn agent_end_without_a_content_length_forwards_none() {
    let wire = r#"{"type":"agent_end","messages":[],"messageRefs":[]}"#;
    let event: AgentEvent = serde_json::from_str(wire).expect("agent_end parses");
    let json = serde_json::to_value(event).expect("serializes");
    assert!(json.get("contentLength").is_none(), "{json}");
}
