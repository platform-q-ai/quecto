//! Extension-related UDS command handlers.
//!
//! Extracted from `uds.rs` to keep file size within the 750-line quality gate.

use super::protocol::AgentEvent;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

pub(super) type ExtensionSnapshot = std::sync::Arc<tokio::sync::RwLock<Vec<serde_json::Value>>>;

/// Build the list of runtime-loadable UDS tools for `get_extensions` responses.
///
/// Historical UDS protocol name retained for compatibility. This is not the
/// complete #1276 native+UDS tool catalogue: bundled native extension tools are
/// reported through tool descriptors/catalogue state and are not governed by UDS
/// load/unload lifecycle.
pub(super) fn build_extension_list(ctx: &DispatchCtx<'_>) -> Vec<serde_json::Value> {
    let ext_names: std::collections::HashSet<String> = ctx
        .agent
        .tool_registry_extension_names()
        .into_iter()
        .collect();
    let descriptors = ctx.agent.tool_descriptors();
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
                let descriptor = descriptors.iter().find(|d| d.name() == name);
                result.push(serde_json::json!({
                    "name": def.name.as_ref(),
                    "description": def.description.as_ref(),
                    "source": descriptor
                        .map(|d| d.source.as_str())
                        .unwrap_or("bundled-native"),
                    "owner": descriptor
                        .map(|d| d.owner.as_ref())
                        .unwrap_or("quecto:official-tools"),
                    "availability": descriptor
                        .map(|d| d.availability.as_str())
                        .unwrap_or("enabled"),
                }));
                covered.insert(name);
            }
        }
    }

    // Include UDS-registered tools not already covered.
    for descriptor in descriptors {
        let name = descriptor.name().to_string();
        if ext_names.contains(&name) && !covered.contains(&name) {
            result.push(serde_json::json!({
                "name": descriptor.definition.name.as_ref(),
                "description": descriptor.definition.description.as_ref(),
                "source": descriptor.source.as_str(),
                "owner": descriptor.owner.as_ref(),
                "availability": descriptor.availability.as_str(),
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
