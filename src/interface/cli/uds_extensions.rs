//! Extension-related UDS command handlers.
//!
//! Extracted from `uds.rs` to keep file size within the 750-line quality gate.

use super::protocol::AgentEvent;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

/// Build the list of registered extension tools for `get_extensions` responses.
///
/// Includes tools from both the `ExtensionRegistry` (native) and
/// UDS-registered tools (from `register_tools` protocol commands).
pub(super) fn build_extension_list(ctx: &DispatchCtx<'_>) -> Vec<serde_json::Value> {
    let ext_names: std::collections::HashSet<String> = ctx
        .agent
        .tool_registry_extension_names()
        .into_iter()
        .collect();
    if ext_names.is_empty() {
        return vec![];
    }

    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result = Vec::new();

    // Include tools from the ExtensionRegistry (native extensions).
    if let Some(ref ext_reg) = ctx.ext_registry {
        let reg = ext_reg.lock().unwrap_or_else(|e| e.into_inner());
        for t in reg.all_tools() {
            let name = t.definition().name.to_string();
            if ext_names.contains(&name) {
                let def = t.definition();
                result.push(serde_json::json!({
                    "name": def.name.as_ref(),
                    "description": def.description.as_ref(),
                }));
                covered.insert(name);
            }
        }
    }

    // Include UDS-registered tools not already covered.
    for def in ctx.agent.tool_definitions() {
        let name = def.name.to_string();
        if ext_names.contains(&name) && !covered.contains(&name) {
            result.push(serde_json::json!({
                "name": def.name.as_ref(),
                "description": def.description.as_ref(),
            }));
        }
    }

    result
}

/// Handle `reload_extensions`: respond with current extension state.
///
/// Since script extensions have been removed (#353), this is now a no-op
/// that returns the current extension list. Native extensions are loaded
/// once at startup; UDS extensions are managed via register/unregister.
pub(super) async fn handle_reload_extensions(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
) {
    let ev = AgentEvent::ok(id, type_name, Some(serde_json::json!({})));
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
}
