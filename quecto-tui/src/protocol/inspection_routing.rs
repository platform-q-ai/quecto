use super::client::Command;

fn subagent_inspection_id(kind: &str, agent_id: &str, id: Option<String>) -> Option<String> {
    let suffix = id.unwrap_or_default();
    Some(format!("subagent-{kind}:{}:{}", agent_id.len(), agent_id,) + ":" + &suffix)
}

pub(crate) fn with_inspection_agent_id(cmd: &Command, agent_id: &str) -> Option<Command> {
    let routed_agent_id = Some(agent_id.to_string());
    Some(match cmd.clone() {
        Command::GetState { id, .. } => Command::GetState {
            id: subagent_inspection_id("state", agent_id, id),
            agent_id: routed_agent_id.clone(),
        },
        Command::GetMessages { id, before, .. } => Command::GetMessages {
            id: subagent_inspection_id("messages", agent_id, id),
            before,
            agent_id: routed_agent_id.clone(),
        },
        Command::GetMessagesTail { id, count, .. } => Command::GetMessagesTail {
            id: subagent_inspection_id("tail", agent_id, id),
            count,
            agent_id: routed_agent_id,
        },
        Command::GetMessage {
            id,
            message_id,
            tool_call_id,
            offset,
            limit,
            ..
        } => Command::GetMessage {
            id: subagent_inspection_id("message", agent_id, id),
            message_id,
            agent_id: routed_agent_id,
            tool_call_id,
            offset,
            limit,
        },
        Command::Sync {
            id,
            epoch,
            since_rev,
            ..
        } => Command::Sync {
            id: subagent_inspection_id("sync", agent_id, id),
            epoch,
            since_rev,
            agent_id: routed_agent_id,
        },
        _ => return None,
    })
}
