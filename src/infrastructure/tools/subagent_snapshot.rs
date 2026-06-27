pub(super) fn response_is_valid_answer(json: &serde_json::Value, command: &str) -> bool {
    let Ok(cmd) = serde_json::from_str::<serde_json::Value>(command) else {
        return false;
    };
    let cmd_type = cmd.get("type").and_then(|v| v.as_str());
    matches!(cmd_type, Some("get_messages" | "get_state"))
        && cmd.get("count").is_none()
        && cmd.get("agent_id").is_none()
        && json.get("command").and_then(|v| v.as_str()) == cmd_type
}
