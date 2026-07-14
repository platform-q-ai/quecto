use super::protocol::AgentCommand;

#[test]
fn get_messages_type_name() {
    let get_messages = AgentCommand::GetMessages {
        id: None,
        count: None,
        before: None,
        agent_id: None,
    };
    assert_eq!(get_messages.type_name(), "get_messages");
}
