use super::{AgentCommand, AgentEvent, TurnMessage, parse_command_line};

#[test]
fn bounded_completion_events_carry_refs_not_content() {
    let event = AgentEvent::TurnEnd {
        message: TurnMessage {
            role: "assistant".into(),
            content: String::new(),
            usage: None,
            stop_reason: None,
            context_tokens: None,
            max_context_tokens: None,
        },
        tool_results: vec![],
        message_refs: vec!["message-1".into()],
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("messageRefs"));
    assert!(!json.contains(&"x".repeat(1024)));
}

#[test]
fn get_message_is_additive_recovery_command() {
    let cmd = parse_command_line(r#"{"type":"get_message","id":"r1","messageRef":"abc"}"#).unwrap();
    assert!(
        matches!(cmd, AgentCommand::GetMessage { ref message_ref, .. } if message_ref == "abc")
    );
    assert_eq!(cmd.type_name(), "get_message");
}
