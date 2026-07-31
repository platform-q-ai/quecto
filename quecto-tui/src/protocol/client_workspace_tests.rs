use super::Event;

#[test]
fn workspace_event_deserializes_1350() {
    let ev: Event = serde_json::from_str(r#"{"type":"workspace","path":"/tmp/ws"}"#).unwrap();
    match ev {
        Event::Workspace { path } => assert_eq!(path, "/tmp/ws"),
        other => panic!("expected workspace event, got {other:?}"),
    }
}
