//! Extension-related UDS command handlers.
//!
//! Extracted from `uds.rs` to keep file size within the 750-line quality gate.

use super::protocol::AgentEvent;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

/// Build the list of registered extension tools for `get_extensions` responses.
///
/// Returns only extensions that are actually registered in the agent's tool
/// registry (shadows of core tools are rejected during registration).
pub(super) fn build_extension_list(ctx: &DispatchCtx<'_>) -> Vec<serde_json::Value> {
    let ext_names: std::collections::HashSet<String> = ctx
        .agent
        .tool_registry_extension_names()
        .into_iter()
        .collect();
    let Some(ref ext_reg) = ctx.ext_registry else {
        return vec![];
    };
    let reg = ext_reg.lock().unwrap();
    reg.all_tools()
        .iter()
        .filter(|t| ext_names.contains(t.definition().name.as_ref()))
        .map(|t| {
            let def = t.definition();
            serde_json::json!({
                "name": def.name.as_ref(),
                "description": def.description.as_ref(),
            })
        })
        .collect()
}

/// Handle `reload_extensions`: re-scan disk, sync tool registry, broadcast event.
pub(super) async fn handle_reload_extensions(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
) {
    let Some(ref ext_reg_arc) = ctx.ext_registry else {
        let ev = AgentEvent::ok(id, type_name, Some(serde_json::json!({})));
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return;
    };

    // 1. Reload script extensions from disk
    {
        let mut ext_reg = ext_reg_arc.lock().unwrap();
        ext_reg.reload_scripts();
    }

    // 2. Build updated extension list for the broadcast event
    let extension_list: Vec<super::protocol::ExtensionInfo> = {
        let ext_reg = ext_reg_arc.lock().unwrap();
        ext_reg
            .all_tools()
            .iter()
            .map(|t| {
                let def = t.definition();
                super::protocol::ExtensionInfo {
                    name: def.name.to_string(),
                    description: def.description.to_string(),
                }
            })
            .collect()
    };

    // 3. Send success response
    let ev = AgentEvent::ok(id, type_name, Some(serde_json::json!({})));
    emit_event_to_broadcast_or_writer(ctx, &ev).await;

    // 4. Broadcast extensions_changed event to all clients
    let ext_changed_ev = AgentEvent::ExtensionsChanged {
        extensions: extension_list,
    };
    emit_event_to_broadcast_or_writer(ctx, &ext_changed_ev).await;
}
