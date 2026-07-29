//! Busy-path interception for sub-agent liveness commands (spike).
//!
//! The dispatch loop is serial: while a parent turn (or auto-continued
//! workflow) is in flight, every queued command waits for it to finish. That
//! starves the two commands the TUI needs to keep child progress live —
//! `get_subagents` (left-panel roster) and child-targeted `sync` (main-panel
//! feed) — so both panels freeze until the parent goes idle (or is aborted).
//!
//! Both are read-only and need nothing the dispatch loop exclusively owns:
//! `get_subagents` reads the `Arc<Mutex<…>>` registry (the #874 connect-time
//! snapshot already does exactly this), and a child-targeted `sync` is a
//! round-trip on the CHILD's socket. Serve them from the connection's reader
//! task, mirroring the #1197 busy-serve pattern for `sync`/`get_message`.

use super::protocol::AgentEvent;

type SubagentRegistry = crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// Intercept `get_subagents` and child-targeted `sync` on the reader task.
/// Returns `true` when the command was fully handled here.
pub(super) async fn intercept(
    line: &str,
    subagents: &Option<SubagentRegistry>,
    clients: &super::uds_ext_protocol::ClientToolRegistry,
    client_id: u64,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return false;
    };
    let id = value.get("id").and_then(|v| v.as_str()).map(str::to_string);
    match value.get("type").and_then(|v| v.as_str()) {
        Some("get_subagents") => {
            // Registry-only read; same data as the dispatch path plus the
            // #842-style snapshot marker (the view may lag the in-flight turn).
            let data = serde_json::json!({
                "subagents": serde_json::to_value(super::protocol::build_subagent_info_list(
                    subagents
                ))
                .unwrap_or_default(),
                "snapshot": true,
            });
            let ev = AgentEvent::ok(id.as_deref(), "get_subagents", Some(data));
            write_event(clients, client_id, id.as_deref(), "get_subagents", &ev).await;
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
    use crate::infrastructure::tools::subagent_registry::{
        INSPECTOR_RESPONSE_TIMEOUT, lookup_subagent_socket, send_subagent_uds_command_with_timeout,
    };
    let Some(registry) = subagents.as_ref() else {
        return AgentEvent::err(id, "sync", "no sub-agent registry available");
    };
    let socket_path = match lookup_subagent_socket(registry, agent_id) {
        Ok(path) => path,
        Err(e) => return AgentEvent::err(id, "sync", e),
    };
    let cmd =
        serde_json::json!({ "type": "sync", "epoch": epoch, "sinceRev": since_rev }).to_string();
    match send_subagent_uds_command_with_timeout(&socket_path, &cmd, INSPECTOR_RESPONSE_TIMEOUT)
        .await
    {
        Ok(line) => match super::uds::uds_forward_response::parse_forwarded_response(&line, "sync")
        {
            Ok(data) => AgentEvent::ok(id, "sync", Some(data)),
            Err(error) => AgentEvent::err(id, "sync", error),
        },
        Err(e) => AgentEvent::err(id, "sync", e.to_string()),
    }
}

/// Write a response line to this client's targeted writer channel, guarding
/// the protocol frame limit the same way the dispatch path does.
async fn write_event(
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
