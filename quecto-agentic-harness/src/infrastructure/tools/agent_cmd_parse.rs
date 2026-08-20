use super::subagent_registry::validate_agent_id_format;

/// Supported commands for interacting with a subagent.
pub(super) const SUPPORTED_COMMANDS: &[&str] = &[
    "prompt",
    "steer",
    "follow_up",
    "abort",
    "kill",
    "get_state",
    "get_messages",
    "get_session_stats",
    "get_subagents",
    "get_subagents_all",
    "get_containers",
    "kill_container",
    "get_tool_catalogue",
    "set_model",
    "set_effort",
    "clear_history",
];

/// Validate the already-parsed arguments and build the JSON command to send.
/// Used by the dispatch path, which parses the arguments once per call.
pub(super) fn build_command(args: &serde_json::Value) -> Result<(String, String, String), String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or("missing required field: command")?
        .to_string();

    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or("missing required field: agent_id")?;

    // Validate agent_id format (same rules as spawn). The synthetic `*` target
    // is accepted only for the parent-local get_subagents_all command.
    if command == "get_subagents_all" {
        if agent_id != "*" {
            return Err("get_subagents_all requires agent_id '*'".to_string());
        }
    } else {
        validate_agent_id_format(&agent_id)?;
    }

    if !SUPPORTED_COMMANDS.contains(&command.as_str()) && command != "get_messages_tail" {
        return Err(format!(
            "unsupported command '{}'; supported: {}",
            command,
            SUPPORTED_COMMANDS.join(", ")
        ));
    }

    // Build the framed JSON command. Control commands (prompt/steer/
    // follow_up/abort) carry `"ack":"accept"` so a BUSY child's reader acks
    // ACCEPTANCE immediately instead of leaving the parent frozen until the
    // child's turn completes (#876); completion still arrives via the
    // passive completion note.
    let json_cmd = match command.as_str() {
        "prompt" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("prompt command requires a message field")?;
            serde_json::json!({"type": "prompt", "message": message, "ack": "accept"})
        }
        "steer" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("steer command requires a message field")?;
            serde_json::json!({"type": "prompt", "message": message, "streamingBehavior": "steer", "ack": "accept"})
        }
        "follow_up" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("follow_up command requires a message field")?;
            serde_json::json!({"type": "follow_up", "message": message, "ack": "accept"})
        }
        "get_state" => {
            let mut cmd = serde_json::json!({"type": "get_state"});
            if let Some(since) = args.get("since").and_then(|v| v.as_u64()) {
                cmd["since"] = serde_json::json!(since);
            }
            cmd
        }
        "get_messages" => {
            let mut cmd = serde_json::json!({"type": "get_messages"});
            match args.get("count") {
                Some(v) if v.is_null() => {}
                Some(v) => {
                    let count = v
                        .as_u64()
                        .ok_or("get_messages count must be a non-negative integer")?;
                    let count = usize::try_from(count)
                        .map_err(|_| "get_messages count is too large".to_string())?;
                    cmd["count"] = serde_json::json!(count);
                }
                None => {}
            }
            // Paged history (#1061): follow a response's `before` cursor to
            // the adjacent older page — an uncounted request returns only
            // the newest bounded page, never the full history.
            match args.get("before") {
                Some(v) if v.is_null() => {}
                Some(v) => {
                    let before = v.as_str().ok_or("get_messages before must be a string")?;
                    cmd["before"] = serde_json::json!(before);
                }
                None => {}
            }
            cmd
        }
        "get_messages_tail" => {
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
            serde_json::json!({"type": "get_messages", "count": count})
        }
        "abort" => serde_json::json!({"type": "abort", "ack": "accept"}),
        "get_session_stats" => serde_json::json!({"type": "get_session_stats"}),
        "set_model" => {
            // Reuse the shared model-arg validation (#881) so `set_model`
            // and `spawn`'s `model` cannot diverge.
            use crate::domain::subagent::{ModelArg, parse_model_arg};
            let parsed = parse_model_arg(
                args.get("model").and_then(|v| v.as_str()),
                args.get("provider").and_then(|v| v.as_str()),
                args.get("model_id").and_then(|v| v.as_str()),
            )
            .map_err(|e| format!("set_model: {e}"))?;
            match parsed {
                Some(ModelArg::Full(m)) => {
                    serde_json::json!({"type": "set_model", "model": m, "ack": "accept"})
                }
                Some(ModelArg::Pair { provider, model_id }) => {
                    serde_json::json!({"type": "set_model", "provider": provider, "modelId": model_id, "ack": "accept"})
                }
                None => {
                    return Err("set_model requires model, or provider + model_id".to_string());
                }
            }
        }
        "set_effort" => {
            let effort = args
                .get("effort")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("set_effort requires effort")?;
            if crate::domain::provider::EffortLevel::parse(effort).is_none() {
                return Err(format!(
                    "invalid effort '{effort}'; valid values: {}",
                    crate::domain::provider::EffortLevel::VALID_VALUES
                ));
            }
            serde_json::json!({"type": "set_effort", "effort": effort, "ack": "accept"})
        }
        "clear_history" => serde_json::json!({"type": "clear_history", "ack": "accept"}),
        "get_subagents" => serde_json::json!({"type": "get_subagents"}),
        "get_subagents_all" => {
            return Err("get_subagents_all is handled locally, not via UDS".to_string());
        }
        "get_tool_catalogue" | "list_tools" => {
            serde_json::json!({"type": "get_tool_catalogue"})
        }
        "kill" => return Err("kill command is handled locally, not via UDS".to_string()),
        _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
    };

    Ok((agent_id, json_cmd.to_string(), command))
}
