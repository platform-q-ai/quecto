//! Busy-path interception for sub-agent roster commands.
//!
//! The dispatch loop is serial: while a parent turn (or auto-continued
//! workflow) is in flight, every queued command waits for it to finish. That
//! starves the commands the TUI needs to keep child state live —
//! `get_subagents` (left-panel roster) and child-targeted `sync` (main-panel
//! feed) — so both panels freeze until the parent goes idle (or is aborted).
//!
//! Neither needs anything the dispatch loop exclusively owns: `get_subagents`
//! reads the `Arc<Mutex<…>>` registry (the #874 connect-time snapshot already
//! does exactly this), and a child-targeted `sync` is a round-trip on the
//! CHILD's socket. Serve them from the connection's reader task, mirroring
//! the #1197 busy-serve pattern for `sync`/`get_message`.
//!
//! `delete_all_subagents` is the one MUTATING command served here (#1626). It
//! only needs the registry mutex and the broadcast sender: the drain is
//! synchronous and lock-safe. Queued behind a running turn it executed only
//! once the parent went idle, while the busy-path `get_subagents` refreshes
//! and child-monitor broadcasts kept re-filling the TUI roster from the
//! untouched registry. Because this interceptor runs before the dispatch
//! channel, multi-client connections never reach the dispatch-path arm.

use super::protocol::AgentEvent;

type SubagentRegistry = crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// Everything the reader task lends the interceptor for one command line.
/// Mirrors `uds_busy_get_message::BusyCommandCtx`.
pub(super) struct BusySubagentCtx<'a> {
    pub line: &'a str,
    pub subagents: &'a Option<SubagentRegistry>,
    /// Shared event fan-out, so a busy-path delete can publish the empty
    /// survivor set to every client (#1626).
    pub broadcast_tx: &'a tokio::sync::broadcast::Sender<String>,
    pub clients: &'a super::uds_ext_protocol::ClientToolRegistry,
    pub client_id: u64,
}

/// Intercept `get_subagents`, `delete_all_subagents` and child-targeted
/// `sync` on the reader task. Returns `true` when the command was fully
/// handled here.
pub(super) async fn intercept(ctx: BusySubagentCtx<'_>) -> bool {
    let BusySubagentCtx {
        line,
        subagents,
        broadcast_tx,
        clients,
        client_id,
    } = ctx;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return false;
    };
    let id = value.get("id").and_then(|v| v.as_str()).map(str::to_string);
    match value.get("type").and_then(|v| v.as_str()) {
        Some("get_subagents") => {
            // Registry-only read; same data as the dispatch path plus the
            // #842-style snapshot marker (the view may lag the in-flight turn).
            let since = match value.get("since") {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => match v.as_u64() {
                    Some(s) => Some(s),
                    None => {
                        let ev =
                            AgentEvent::err(id.as_deref(), "get_subagents", "invalid since cursor");
                        write_event(clients, client_id, id.as_deref(), "get_subagents", &ev).await;
                        return true;
                    }
                },
            };
            let mut data = match super::protocol::build_compact_subagent_roster(subagents, since)
                .and_then(|roster| serde_json::to_value(roster).map_err(|e| e.to_string()))
            {
                Ok(data) => data,
                Err(e) => {
                    let ev = AgentEvent::err(id.as_deref(), "get_subagents", &e);
                    write_event(clients, client_id, id.as_deref(), "get_subagents", &ev).await;
                    return true;
                }
            };
            if let Some(obj) = data.as_object_mut() {
                obj.insert("snapshot".to_string(), serde_json::Value::Bool(true));
            }
            let ev = AgentEvent::ok(id.as_deref(), "get_subagents", Some(data));
            write_event(clients, client_id, id.as_deref(), "get_subagents", &ev).await;
            true
        }
        Some("delete_all_subagents") => {
            // Drain + signal + broadcast the empty survivor set synchronously
            // (#1626): the TUI cleared its roster optimistically at send time,
            // so the response must not wait for the in-flight turn.
            let ev = super::uds_delete_all_subagents::busy_response(
                subagents.as_ref(),
                broadcast_tx,
                id.as_deref(),
            );
            write_event(
                clients,
                client_id,
                id.as_deref(),
                "delete_all_subagents",
                &ev,
            )
            .await;
            true
        }
        Some("get_report" | "get_state") if value.get("agent_id").is_some() => {
            let Ok(command) = serde_json::from_str::<super::protocol::AgentCommand>(line) else {
                return false;
            };
            let Some(agent_id) = value["agent_id"].as_str().map(str::to_owned) else {
                return false;
            };
            let command_type = command.type_name().to_owned();
            // Forwarders run concurrently (replies may reorder relative to
            // queued commands) but are bounded per process so a client cannot
            // fan out unlimited in-flight descendant queries.
            static FORWARDERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
            let Ok(permit) = FORWARDERS.try_acquire() else {
                let event = super::protocol::AgentEvent::err(
                    id.as_deref(),
                    &command_type,
                    "descendant query capacity exhausted; retry after in-flight queries return",
                );
                write_event(clients, client_id, id.as_deref(), &command_type, &event).await;
                return true;
            };
            let subagents = subagents.clone();
            let clients = clients.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let event =
                    forward_child_command(&subagents, id.as_deref(), &agent_id, value).await;
                write_event(&clients, client_id, id.as_deref(), &command_type, &event).await;
            });
            true
        }
        Some("sync") if value.get("agent_id").is_some() => {
            let (Some(agent_id), Some(epoch), Some(since_rev)) = (
                value.get("agent_id").and_then(|v| v.as_str()),
                value.get("epoch").and_then(|v| v.as_u64()),
                value.get("sinceRev").and_then(|v| v.as_u64()),
            ) else {
                // Malformed child sync: fall through so the dispatch loop
                // produces its usual parse/validation error.
                return false;
            };
            // The child round-trip can take up to the inspector timeout; run it
            // detached so a slow child never blocks this connection's reader
            // (which must stay responsive for abort/steer).
            let subagents = subagents.clone();
            let clients = clients.clone();
            let agent_id = agent_id.to_string();
            tokio::spawn(async move {
                let ev = forward_child_sync(&subagents, id.as_deref(), &agent_id, epoch, since_rev)
                    .await;
                write_event(&clients, client_id, id.as_deref(), "sync", &ev).await;
            });
            true
        }
        _ => false,
    }
}

/// Round-trip a `sync` on the child's own socket and wrap the reply as this
/// command's response — same shape as the dispatch-path
/// `uds_dispatch_sync_forward::forward_subagent_sync`.
async fn forward_child_sync(
    subagents: &Option<SubagentRegistry>,
    id: Option<&str>,
    agent_id: &str,
    epoch: u64,
    since_rev: u64,
) -> AgentEvent {
    forward_child_command(
        subagents,
        id,
        agent_id,
        serde_json::json!({ "type":"sync", "epoch":epoch, "sinceRev":since_rev }),
    )
    .await
}

/// Forward an already validated command to its owning descendant without running a model turn.
pub(super) async fn forward_child_command(
    subagents: &Option<SubagentRegistry>,
    id: Option<&str>,
    agent_id: &str,
    mut cmd: serde_json::Value,
) -> AgentEvent {
    let command_type = cmd["type"]
        .as_str()
        .expect("validated command type")
        .to_owned();
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, send_subagent_uds_command_with_timeout,
    };
    use crate::infrastructure::tools::subagent_routing::{
        InspectionRoute, resolve_inspection_route,
    };
    let Some(registry) = subagents.as_ref() else {
        return AgentEvent::err(id, &command_type, "no sub-agent registry available");
    };
    let route = match resolve_inspection_route(registry, agent_id) {
        Ok(route) => route,
        Err(e) => return AgentEvent::err(id, &command_type, e),
    };
    cmd.as_object_mut()
        .expect("validated command object")
        .remove("agent_id");
    cmd["id"] = serde_json::json!(id);
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
        Ok(line) => {
            match super::uds::uds_forward_response::parse_forwarded_response(&line, &command_type) {
                Ok(data) => AgentEvent::ok(id, &command_type, Some(data)),
                Err(error) => AgentEvent::err(id, &command_type, error),
            }
        }
        Err(e) => AgentEvent::err(id, &command_type, e.to_string()),
    }
}

/// Write a response line to this client's targeted writer channel, guarding
/// the protocol frame limit the same way the dispatch path does.
pub(super) async fn write_event(
    clients: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
    id: Option<&str>,
    command: &str,
    ev: &AgentEvent,
) {
    if let Some(tx) = super::uds_ext_protocol::client_writer_tx(clients, client_id) {
        let mut response = serde_json::to_string(ev).unwrap_or_default();
        if response.len() > crate::infrastructure::line_cap::EVENT_LINE_JSON_BUDGET {
            response = serde_json::to_string(&AgentEvent::err(
                id,
                command,
                "response exceeds the protocol frame limit",
            ))
            .unwrap_or_default();
        }
        response.push('\n');
        let _ = tx.send(response).await;
    }
}
