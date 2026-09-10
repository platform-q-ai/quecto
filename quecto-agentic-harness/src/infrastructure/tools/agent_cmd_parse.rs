use super::subagent_registry::validate_agent_id_format;

/// Supported commands for interacting with a subagent.
pub(super) const SUPPORTED_COMMANDS: &[&str] = &[
    "swarm_control",
    "prompt",
    "steer",
    "follow_up",
    "abort",
    "kill",
    "get_state",
    "get_report",
    "get_messages",
    "get_message",
    "get_session_stats",
    "get_subagents",
    "get_subagents_all",
    "get_containers",
    "kill_container",
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

    if !SUPPORTED_COMMANDS.contains(&command.as_str())
        && command != "get_messages_tail"
        && command != "get_tool_catalogue"
        && command != "list_tools"
    {
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
        "swarm_control" => {
            let action = args["action"]
                .as_str()
                .filter(|action| {
                    matches!(
                        *action,
                        "pause" | "resume" | "close" | "extend" | "status" | "usage_budget"
                    )
                })
                .ok_or(
                    "swarm_control action must be pause, resume, close, extend, status or usage_budget",
                )?;
            let mut command = serde_json::json!({"type":"swarm_control","action":action});
            if action == "extend" {
                let seconds = args
                    .get("deadline_seconds")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|seconds| *seconds > 0)
                    .ok_or("deadline_seconds must be a positive integer for extend")?;
                command["deadline_seconds"] = serde_json::json!(seconds);
            }
            if action == "usage_budget" {
                let limit = args
                    .get("token_limit")
                    .ok_or("token_limit is required; null disables the budget")?;
                if limit.is_null() || limit.as_u64().is_some_and(|limit| limit > 0) {
                    command["token_limit"] = limit.clone();
                } else {
                    return Err("token_limit must be positive or null".into());
                }
                command["strict_unknown"] = serde_json::json!(match args.get("strict_unknown") {
                    None => true,
                    Some(value) => value.as_bool().ok_or("strict_unknown must be boolean")?,
                });
            }
            if let Some(reason) = args.get("reason") {
                command["reason"] =
                    serde_json::json!(reason.as_str().ok_or("reason must be a string")?);
            }
            command
        }
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
        "get_report" => {
            let export_raw = match args.get("export_raw") {
                None => false,
                Some(value) => value.as_bool().ok_or("export_raw must be boolean")?,
            };
            serde_json::json!({"type":"get_report", "export_raw":export_raw})
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
        "get_message" => {
            let message_id = args
                .get("messageId")
                .or_else(|| args.get("message_id"))
                .and_then(|v| v.as_str())
                .ok_or("get_message requires messageId")?;
            let mut cmd = serde_json::json!({"type": "get_message", "messageId": message_id});
            if let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) {
                cmd["agent_id"] = serde_json::json!(agent_id);
            }
            if let Some(tool_call_id) = args
                .get("toolCallId")
                .or_else(|| args.get("tool_call_id"))
                .and_then(|v| v.as_str())
            {
                cmd["toolCallId"] = serde_json::json!(tool_call_id);
            }
            for field in ["offset", "limit"] {
                match args.get(field) {
                    Some(v) if v.is_null() => {}
                    Some(v) => {
                        let n = v.as_u64().ok_or_else(|| {
                            format!("get_message {field} must be a non-negative integer")
                        })?;
                        let n = usize::try_from(n)
                            .map_err(|_| format!("get_message {field} is too large"))?;
                        cmd[field] = serde_json::json!(n);
                    }
                    None => {}
                }
            }
            cmd
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
        "get_subagents" => {
            let mut cmd = serde_json::json!({"type": "get_subagents"});
            if let Some(v) = args.get("since") {
                if !v.is_null() {
                    cmd["since"] = serde_json::json!(
                        v.as_u64()
                            .ok_or("get_subagents since must be a non-negative integer")?
                    );
                }
            }
            cmd
        }
        "get_subagents_all" => {
            return Err("get_subagents_all is handled locally, not via UDS".to_string());
        }
        "get_tool_catalogue" | "list_tools" => {
            return Err(
                "get_tool_catalogue/list_tools is not available via model-facing agent_cmd"
                    .to_string(),
            );
        }
        "kill" => return Err("kill command is handled locally, not via UDS".to_string()),
        _ => unreachable!(), // Covered by SUPPORTED_COMMANDS check above.
    };

    Ok((agent_id, json_cmd.to_string(), command))
}
