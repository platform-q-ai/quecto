use super::client::Command;

fn subagent_inspection_id(
    ns: &str,
    kind: &str,
    agent_id: &str,
    id: Option<String>,
) -> Option<String> {
    let suffix = id.unwrap_or_default();
    Some(format!("{ns}subagent-{kind}:{}:{}", agent_id.len(), agent_id,) + ":" + &suffix)
}

/// `ns` is the owning tab's connection namespace (`tab{N}:`, #1463): minted
/// ids carry it so broadcast responses can never match another tab's feeds.
/// ONE routable-variant list serves both routing and namespacing (#1472 r1).
/// Whether `cmd` is a routable inspection command — the side-effect-free
/// gate for inspection-only feeds (#1472 r2: no Command clone, and no
/// empty-namespace sentinel that could leak un-namespaced ids).
pub(crate) fn is_inspection_routable(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::GetState { .. }
            | Command::GetMessages { .. }
            | Command::GetMessagesTail { .. }
            | Command::GetMessage { .. }
            | Command::Sync { .. }
    )
}

pub(crate) fn with_inspection_agent_id(cmd: &Command, agent_id: &str, ns: &str) -> Option<Command> {
    let routed_agent_id = Some(agent_id.to_string());
    Some(match cmd.clone() {
        Command::GetState { id, .. } => Command::GetState {
            id: subagent_inspection_id(ns, "state", agent_id, id),
            agent_id: routed_agent_id.clone(),
        },
        Command::GetMessages { id, before, .. } => Command::GetMessages {
            id: subagent_inspection_id(ns, "messages", agent_id, id),
            before,
            agent_id: routed_agent_id.clone(),
        },
        Command::GetMessagesTail { id, count, .. } => Command::GetMessagesTail {
            id: subagent_inspection_id(ns, "tail", agent_id, id),
            count,
            agent_id: routed_agent_id,
        },
        Command::GetMessage {
            id,
            message_id,
            tool_call_id,
            offset,
            thinking_offset,
            limit,
            ..
        } => Command::GetMessage {
            id: subagent_inspection_id(ns, "message", agent_id, id),
            message_id,
            agent_id: routed_agent_id,
            tool_call_id,
            offset,
            thinking_offset,
            limit,
        },
        Command::Sync {
            id,
            epoch,
            since_rev,
            ..
        } => Command::Sync {
            id: subagent_inspection_id(ns, "sync", agent_id, id),
            epoch,
            since_rev,
            agent_id: routed_agent_id,
        },
        _ => return None,
    })
}
