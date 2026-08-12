use super::*;

#[test]
fn sync_command_includes_epoch_and_since_revision() {
    let cmd = Command::Sync {
        agent_id: None,
        id: Some("sync-1".into()),
        epoch: 2,
        since_rev: 3,
    };
    let value = serde_json::to_value(&cmd).unwrap();
    assert_eq!(value["type"], "sync");
    assert_eq!(value["id"], "sync-1");
    assert_eq!(value["epoch"], 2);
    assert_eq!(value["sinceRev"], 3);
}

#[test]
fn event_deserializes_ledger_advanced_hint() {
    let event: Event =
        serde_json::from_str(r#"{"type":"ledger_advanced","epoch":4,"rev":5}"#).unwrap();
    assert!(matches!(event, Event::LedgerAdvanced { epoch: 4, rev: 5 }));
}
