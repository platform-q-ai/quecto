use super::{AgentCommand, parse_command_line};

#[test]
fn get_tool_catalogue_command_serializes() {
    let cmd = AgentCommand::GetToolCatalogue {
        id: Some("tc-1".into()),
    };
    let j = serde_json::to_value(&cmd).unwrap();
    assert_eq!(j["type"], "get_tool_catalogue");
    assert_eq!(j["id"], "tc-1");
}

#[test]
fn list_tools_alias_parses_to_get_tool_catalogue() {
    let cmd = parse_command_line(r#"{"type":"list_tools","id":"lt-1"}"#).unwrap();
    assert_eq!(cmd.type_name(), "get_tool_catalogue");
    assert_eq!(cmd.id(), Some("lt-1"));
}

#[test]
fn get_tool_catalogue_parses_without_id() {
    let cmd = parse_command_line(r#"{"type":"get_tool_catalogue"}"#).unwrap();
    assert_eq!(cmd.type_name(), "get_tool_catalogue");
    assert_eq!(cmd.id(), None);
}
