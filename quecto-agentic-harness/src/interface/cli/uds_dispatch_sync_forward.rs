use super::super::protocol::AgentEvent;
use super::DispatchCtx;

pub(super) async fn forward_subagent_sync(
    ctx: &DispatchCtx<'_>,
    id: Option<&str>,
    tn: &str,
    agent_id: &str,
    epoch: u64,
    since_rev: u64,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id, tn, "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, agent_id) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id, tn, e),
    };
    let cmd =
        serde_json::json!({ "type": "sync", "epoch": epoch, "sinceRev": since_rev }).to_string();
    match send_subagent_uds_command_with_timeout(&socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        Ok(line) => match super::uds_forward_response::parse_forwarded_response(&line, "sync") {
            Ok(data) => AgentEvent::ok(id, tn, Some(data)),
            Err(error) => AgentEvent::err(id, tn, error),
        },
        Err(e) => AgentEvent::err(id, tn, e.to_string()),
    }
}
