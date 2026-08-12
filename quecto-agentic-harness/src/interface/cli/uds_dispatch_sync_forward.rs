use super::super::protocol::AgentEvent;
use super::DispatchCtx;
use crate::domain::ids::{AgentId, CommandId};

pub(super) async fn forward_subagent_sync(
    ctx: &DispatchCtx<'_>,
    id: Option<CommandId>,
    tn: &str,
    agent_id: AgentId,
    epoch: u64,
    since_rev: u64,
) -> AgentEvent {
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, send_subagent_uds_command_with_timeout,
    };
    use crate::infrastructure::tools::subagent_routing::{
        InspectionRoute, resolve_inspection_route,
    };
    let id_ref = id.as_ref().map(CommandId::as_str);
    let Some(registry) = ctx.subagent_registry.as_ref() else {
        return AgentEvent::err(id_ref, tn, "no sub-agent registry available");
    };
    let route = match resolve_inspection_route(registry, agent_id.as_str()) {
        Ok(route) => route,
        Err(e) => return AgentEvent::err(id_ref, tn, e),
    };
    let mut cmd = serde_json::json!({ "type": "sync", "epoch": epoch, "sinceRev": since_rev });
    if let InspectionRoute::ViaAncestor { target_id, .. } = &route {
        cmd["agent_id"] = serde_json::json!(target_id);
    }
    let socket_path = match &route {
        InspectionRoute::Direct { socket_path } => socket_path,
        InspectionRoute::ViaAncestor {
            ancestor_socket_path,
            ..
        } => ancestor_socket_path,
    };
    let cmd = cmd.to_string();
    match send_subagent_uds_command_with_timeout(socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        Ok(line) => match super::uds_forward_response::parse_forwarded_response(&line, "sync") {
            Ok(data) => AgentEvent::ok(id_ref, tn, Some(data)),
            Err(error) => AgentEvent::err(id_ref, tn, error),
        },
        Err(e) => AgentEvent::err(id_ref, tn, e.to_string()),
    }
}
