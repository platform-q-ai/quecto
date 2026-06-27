pub(super) fn response_is_valid_answer(json: &serde_json::Value, command: &str) -> bool {
    if json.get("id").is_some() {
        return false;
    }
    let Ok(cmd) = serde_json::from_str::<serde_json::Value>(command) else {
        return false;
    };
    let cmd_type = cmd.get("type").and_then(|v| v.as_str());
    if cmd.get("count").is_some() || cmd.get("agent_id").is_some() {
        return false;
    }
    match cmd_type {
        Some("get_messages") => {
            json.get("command").and_then(|v| v.as_str()) == Some("get_messages")
                && json
                    .pointer("/data/messages")
                    .and_then(|v| v.as_array())
                    .is_some()
        }
        Some("get_state") => {
            json.get("command").and_then(|v| v.as_str()) == Some("get_state")
                && json
                    .pointer("/data/isStreaming")
                    .and_then(|v| v.as_bool())
                    .is_some()
                && json
                    .pointer("/data/messageCount")
                    .and_then(|v| v.as_u64())
                    .is_some()
        }
        _ => false,
    }
}
