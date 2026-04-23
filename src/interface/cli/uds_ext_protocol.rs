// UDS extension protocol handlers (#352).
//
// Handles register_tools, unregister_tools, and tool_result commands from
// extension clients connected via the UDS socket.
//
// Key design: each client gets a `ClientToolState` that tracks:
// - Which tool names this client has registered
// - Pending tool execution results (tool_call_id → oneshot sender)
//
// When a client disconnects, all its tools are unregistered and any
// pending executions receive an error result.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::extensions::uds_tool::{UdsToolRequest, create_uds_tool};

use super::protocol::{AgentEvent, ExtensionInfo, ToolRegistration};

/// Default timeout for UDS extension tool execution (seconds).
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 30;

/// Per-client tool state.
#[derive(Debug, Default)]
pub struct ClientToolState {
    /// Tool names registered by this client.
    pub tool_names: HashSet<String>,
    /// Pending tool execution results: tool_call_id → result sender. The
    /// forwarder task below populates this when it dispatches an
    /// `execute_tool`; `handle_tool_result` drains it when the client's
    /// `tool_result` arrives.
    pub pending_results: HashMap<String, tokio::sync::oneshot::Sender<ToolResult>>,
    /// Receivers for incoming execution requests (one per tool). Staged
    /// here by `handle_register_tools` until `dispatch_register_tools`
    /// takes ownership and spawns a forwarder task for each — at which
    /// point the rx is moved out and replaced by an entry in
    /// `tool_request_tasks`.
    pub tool_request_rxs: HashMap<String, tokio::sync::mpsc::Receiver<UdsToolRequest>>,
    /// Forwarder tasks that consume `UdsToolRequest`s from the mpsc
    /// receiver created by `create_uds_tool`, register the oneshot
    /// result sender into `pending_results`, and emit an `execute_tool`
    /// event to the wire. Keyed by tool name. Aborted on unregister or
    /// client disconnect.
    pub tool_request_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

/// Shared registry of per-client tool state, keyed by client ID.
///
/// The `Mutex` is held briefly during register/unregister operations.
/// Tool execution itself is async and does not hold the mutex.
pub type ClientToolRegistry = Arc<Mutex<HashMap<u64, ClientToolState>>>;

/// Create a new empty client tool registry.
pub fn new_client_tool_registry() -> ClientToolRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Arguments for `handle_register_tools`.
pub struct RegisterToolsArgs<'a> {
    pub client_id: u64,
    pub id: Option<&'a str>,
    pub tools: &'a [ToolRegistration],
    pub registry: &'a ClientToolRegistry,
    pub core_tool_names: &'a HashSet<String>,
}

/// Handle `register_tools` command from a client.
///
/// Returns `(success, response_event)` — caller emits the response.
/// On success, also returns a list of `Arc<dyn Tool>` to register.
pub fn handle_register_tools(
    args: RegisterToolsArgs<'_>,
) -> (bool, AgentEvent, Vec<Arc<dyn crate::domain::tool::Tool>>) {
    let RegisterToolsArgs {
        client_id,
        id,
        tools,
        registry,
        core_tool_names,
    } = args;
    // Check for shadow of core tools.
    for tool in tools {
        if core_tool_names.contains(&tool.name) {
            let ev = AgentEvent::err(
                id,
                "register_tools",
                format!("tool '{}' shadows a core tool", tool.name),
            );
            return (false, ev, vec![]);
        }
    }

    let timeout = std::time::Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS);
    let mut new_tools: Vec<Arc<dyn crate::domain::tool::Tool>> = Vec::new();
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    let state = reg.entry(client_id).or_default();

    for tool_reg in tools {
        let def = ToolDefinition {
            name: tool_reg.name.clone().into(),
            description: tool_reg.description.clone().into(),
            parameters_schema: tool_reg.parameters_schema.clone().into(),
        };

        // If tool was already registered by this client, unregister old one first.
        if state.tool_names.contains(&tool_reg.name) {
            state.tool_request_rxs.remove(&tool_reg.name);
        }

        let (tool, rx) = create_uds_tool(def, timeout);
        state.tool_names.insert(tool_reg.name.clone());
        state.tool_request_rxs.insert(tool_reg.name.clone(), rx);
        new_tools.push(tool);
    }

    let registered: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
    let ev = AgentEvent::ok(
        id,
        "register_tools",
        Some(serde_json::json!({ "registered": registered })),
    );
    (true, ev, new_tools)
}

/// Handle `unregister_tools` command from a client.
///
/// Returns the names that were actually removed (for caller to unregister
/// from the tool registry).
pub fn handle_unregister_tools(
    client_id: u64,
    id: Option<&str>,
    tool_names: &[String],
    registry: &ClientToolRegistry,
) -> (AgentEvent, Vec<String>) {
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    let mut removed = Vec::new();

    if let Some(state) = reg.get_mut(&client_id) {
        for name in tool_names {
            if state.tool_names.remove(name) {
                state.tool_request_rxs.remove(name);
                if let Some(handle) = state.tool_request_tasks.remove(name) {
                    handle.abort();
                }
                removed.push(name.clone());
            }
        }
    }

    let ev = AgentEvent::ok(
        id,
        "unregister_tools",
        Some(serde_json::json!({ "unregistered": removed })),
    );
    (ev, removed)
}

/// Arguments for `handle_tool_result`.
pub struct ToolResultArgs<'a> {
    pub client_id: u64,
    pub tool_call_id: &'a str,
    pub content: &'a str,
    pub is_error: bool,
    pub registry: &'a ClientToolRegistry,
}

/// Handle `tool_result` command from a client.
pub fn handle_tool_result(args: ToolResultArgs<'_>) {
    let ToolResultArgs {
        client_id,
        tool_call_id,
        content,
        is_error,
        registry,
    } = args;
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = reg.get_mut(&client_id) {
        if let Some(tx) = state.pending_results.remove(tool_call_id) {
            let _ = tx.send(ToolResult {
                content: content.to_string(),
                is_error,
                image_blocks: vec![],
            });
        }
    }
}

/// Handle client disconnect: unregister all tools, cancel pending executions.
///
/// Returns the names of tools that were removed (for caller to unregister
/// from the tool registry).
pub fn handle_client_disconnect(client_id: u64, registry: &ClientToolRegistry) -> Vec<String> {
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = reg.remove(&client_id) else {
        return vec![];
    };

    // Cancel all pending tool executions.
    for (_, tx) in state.pending_results {
        let _ = tx.send(ToolResult {
            content: "Extension disconnected".to_string(),
            is_error: true,
            image_blocks: vec![],
        });
    }

    // Abort any forwarder tasks owned by the disconnecting client —
    // dropping their mpsc receivers would let them exit naturally, but
    // explicit abort shaves a small amount of background work.
    for (_, handle) in state.tool_request_tasks {
        handle.abort();
    }

    state.tool_names.into_iter().collect()
}

/// Build the current extension list from the agent's tool registry.
pub fn build_extensions_changed_event(
    extension_names: &[String],
    agent: &crate::application::agent_loop::AgentLoopImpl,
) -> AgentEvent {
    let extensions: Vec<ExtensionInfo> = extension_names
        .iter()
        .filter_map(|name| {
            // Look up the tool definition for the description.
            let defs = agent.tool_definitions();
            defs.iter()
                .find(|d| d.name.as_ref() == name)
                .map(|d| ExtensionInfo {
                    name: d.name.to_string(),
                    description: d.description.to_string(),
                })
        })
        .collect();
    AgentEvent::ExtensionsChanged { extensions }
}

// ─── Dispatch helpers (called from uds.rs dispatch_command) ───────────────

/// Handle `register_tools` command in dispatch context.
pub(super) async fn dispatch_register_tools(
    ctx: &mut super::uds::DispatchCtx<'_>,
    id: Option<&str>,
    tools: &[ToolRegistration],
) {
    let ext_names = ctx.agent.tool_registry_extension_names();
    let core_names: std::collections::HashSet<String> = ctx
        .agent
        .tool_definitions()
        .iter()
        .filter(|d| !ext_names.contains(&d.name.to_string()))
        .map(|d| d.name.to_string())
        .collect();

    let (ok, ev, new_tools) = handle_register_tools(RegisterToolsArgs {
        client_id: ctx.current_client_id,
        id,
        tools,
        registry: &ctx.client_tool_registry,
        core_tool_names: &core_names,
    });
    super::uds::emit_event_to_broadcast_or_writer(ctx, &ev).await;

    if ok && !new_tools.is_empty() {
        for tool in &new_tools {
            ctx.agent.register_extension_tool(tool.clone());
        }
        // Spawn a forwarder task for each newly-registered tool. These
        // drain the mpsc receiver stored in `tool_request_rxs` and are
        // the reason tool calls from the LLM actually reach the
        // extension client as `execute_tool` events.
        for tool_reg in tools {
            spawn_tool_forwarder_for(ctx, &tool_reg.name);
        }
        let ext_names = ctx.agent.tool_registry_extension_names();
        let changed = build_extensions_changed_event(&ext_names, ctx.agent);
        super::uds::emit_event_to_broadcast_or_writer(ctx, &changed).await;
    }
}

/// Take the just-staged receiver for `tool_name` out of the client's
/// `tool_request_rxs`, spawn a forwarder task that emits `execute_tool`
/// events and parks `result_tx` senders into `pending_results`, and
/// record the task handle for abort on unregister/disconnect.
fn spawn_tool_forwarder_for(
    ctx: &mut super::uds::DispatchCtx<'_>,
    tool_name: &str,
) {
    let client_id = ctx.current_client_id;
    let registry = ctx.client_tool_registry.clone();
    let broadcast_tx = ctx.broadcast_tx.clone();

    let rx_opt = {
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        let state = match reg.get_mut(&client_id) {
            Some(s) => s,
            None => return,
        };
        // Abort any forwarder for the same tool (re-register case).
        if let Some(old) = state.tool_request_tasks.remove(tool_name) {
            old.abort();
        }
        state.tool_request_rxs.remove(tool_name)
    };
    let Some(rx) = rx_opt else {
        return;
    };

    let name_for_task = tool_name.to_string();
    let handle = tokio::spawn(forward_tool_requests(
        client_id,
        name_for_task,
        rx,
        registry.clone(),
        broadcast_tx,
    ));

    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = reg.get_mut(&client_id) {
        state.tool_request_tasks.insert(tool_name.to_string(), handle);
    } else {
        // Client disappeared between the two locks. Dropping the
        // handle aborts it; tool exec will surface "Extension
        // disconnected" via the existing UdsTool path.
        handle.abort();
    }
}

/// Drain `UdsToolRequest`s for a single (client, tool) pair: stash the
/// oneshot result sender in `pending_results` so `tool_result` handlers
/// can resolve it, then emit an `execute_tool` event to the wire.
///
/// Broadcast-wide event is fine because `execute_tool` is scoped by
/// `tool_name` — only the client that registered the tool has a
/// dispatcher for it.  Other connected clients see the event, find no
/// matching handler, and ignore it.  This avoids adding a per-client
/// event channel just for one event type.
async fn forward_tool_requests(
    client_id: u64,
    tool_name: String,
    mut rx: tokio::sync::mpsc::Receiver<UdsToolRequest>,
    registry: ClientToolRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
) {
    while let Some(req) = rx.recv().await {
        let UdsToolRequest {
            tool_call_id,
            tool_name: sent_tool,
            arguments,
            result_tx,
        } = req;

        // Dispatch bugs would show up here as a mismatch between the
        // tool the UdsTool thinks it's calling and the one we're
        // forwarding for. Catch it loudly in debug builds; in release
        // we continue using the forwarder's own `tool_name` (the one
        // keyed into this task at registration).
        debug_assert_eq!(
            sent_tool, tool_name,
            "forwarder tool_name mismatch: registered={tool_name:?} request={sent_tool:?}"
        );

        // Stash the oneshot in `pending_results` BEFORE broadcasting
        // `execute_tool` so the reader task's `handle_tool_result`
        // (which takes the same registry mutex) always finds the
        // pending entry when the client responds — even on the fastest
        // possible local-socket round-trip.
        {
            let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
            match reg.get_mut(&client_id) {
                Some(state) => {
                    state.pending_results.insert(tool_call_id.clone(), result_tx);
                }
                None => {
                    // Client gone — drop the request; the UdsTool
                    // timeout path surfaces a clean error upstream.
                    continue;
                }
            }
        }

        let ev = super::protocol::AgentEvent::ExecuteTool {
            tool_call_id: tool_call_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
        };
        if let Some(ref tx) = broadcast_tx {
            let mut line = ev.to_json_line();
            line.push('\n');
            let _ = tx.send(line);
        } else {
            // Single-client test mode — nobody to write to.  Remove
            // the pending entry so the UdsTool times out cleanly
            // rather than leaking.
            let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = reg.get_mut(&client_id) {
                state.pending_results.remove(&tool_call_id);
            }
        }
    }
}

/// Handle `unregister_tools` command in dispatch context.
pub(super) async fn dispatch_unregister_tools(
    ctx: &mut super::uds::DispatchCtx<'_>,
    id: Option<&str>,
    tool_names: &[String],
) {
    let (ev, removed) = handle_unregister_tools(
        ctx.current_client_id,
        id,
        tool_names,
        &ctx.client_tool_registry,
    );
    super::uds::emit_event_to_broadcast_or_writer(ctx, &ev).await;

    if !removed.is_empty() {
        for name in &removed {
            ctx.agent.unregister_extension_tool(name);
        }
        let ext_names = ctx.agent.tool_registry_extension_names();
        let changed = build_extensions_changed_event(&ext_names, ctx.agent);
        super::uds::emit_event_to_broadcast_or_writer(ctx, &changed).await;
    }
}

/// Handle `tool_result` command in dispatch context.
pub(super) fn dispatch_tool_result(
    ctx: &mut super::uds::DispatchCtx<'_>,
    tool_call_id: &str,
    content: &str,
    is_error: bool,
) {
    handle_tool_result(ToolResultArgs {
        client_id: ctx.current_client_id,
        tool_call_id,
        content,
        is_error,
        registry: &ctx.client_tool_registry,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_names() -> HashSet<String> {
        ["bash", "read", "write", "edit", "ls", "grep", "find"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn tool_reg(name: &str, desc: &str) -> ToolRegistration {
        ToolRegistration {
            name: name.into(),
            description: desc.into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn reg(
        registry: &ClientToolRegistry,
        core: &HashSet<String>,
        cid: u64,
        tools: &[ToolRegistration],
    ) -> (
        bool,
        AgentEvent,
        Vec<std::sync::Arc<dyn crate::domain::tool::Tool>>,
    ) {
        handle_register_tools(RegisterToolsArgs {
            client_id: cid,
            id: None,
            tools,
            registry,
            core_tool_names: core,
        })
    }

    /// Test context for register operations.
    struct RegCtx {
        registry: ClientToolRegistry,
        core: HashSet<String>,
    }

    impl RegCtx {
        fn new() -> Self {
            Self {
                registry: new_client_tool_registry(),
                core: core_names(),
            }
        }

        fn register_id(
            &self,
            cid: u64,
            id: &str,
            tools: &[ToolRegistration],
        ) -> (
            bool,
            AgentEvent,
            Vec<std::sync::Arc<dyn crate::domain::tool::Tool>>,
        ) {
            handle_register_tools(RegisterToolsArgs {
                client_id: cid,
                id: Some(id),
                tools,
                registry: &self.registry,
                core_tool_names: &self.core,
            })
        }
    }

    #[test]
    fn test_register_tools_success() {
        let ctx = RegCtx::new();
        let (ok, ev, tools) = ctx.register_id(1, "rt-1", &[tool_reg("weather", "Get weather")]);
        assert!(ok);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].definition().name.as_ref(), "weather");
        assert!(ev.to_json_line().contains("\"success\":true"));
    }

    #[test]
    fn test_register_tools_rejects_core_shadow() {
        let ctx = RegCtx::new();
        let (ok, ev, tools) = ctx.register_id(1, "rt-2", &[tool_reg("bash", "Shadow bash")]);
        assert!(!ok);
        assert!(tools.is_empty());
        let json = ev.to_json_line();
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("shadows a core tool"));
    }

    #[test]
    fn test_register_tools_multiple() {
        let r = new_client_tool_registry();
        let c = core_names();
        let tools_arr = [tool_reg("weather", "W"), tool_reg("translate", "T")];
        let (ok, _, tools) = reg(&r, &c, 1, &tools_arr);
        assert!(ok);
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_register_tools_idempotent() {
        let r = new_client_tool_registry();
        let c = core_names();
        let (ok, _, _) = reg(&r, &c, 1, &[tool_reg("weather", "Old desc")]);
        assert!(ok);
        let (ok, _, tools) = reg(&r, &c, 1, &[tool_reg("weather", "New desc")]);
        assert!(ok);
        assert_eq!(tools[0].definition().description.as_ref(), "New desc");
        assert_eq!(r.lock().unwrap()[&1].tool_names.len(), 1);
    }

    #[test]
    fn test_unregister_tools() {
        let r = new_client_tool_registry();
        let c = core_names();
        reg(&r, &c, 1, &[tool_reg("weather", "Get weather")]);
        let (ev, removed) = handle_unregister_tools(1, Some("ut-1"), &["weather".into()], &r);
        assert_eq!(removed, vec!["weather"]);
        assert!(ev.to_json_line().contains("\"success\":true"));
        assert!(r.lock().unwrap()[&1].tool_names.is_empty());
    }

    #[test]
    fn test_unregister_tools_unknown_name_is_noop() {
        let r = new_client_tool_registry();
        let (_, removed) = handle_unregister_tools(1, None, &["nonexistent".into()], &r);
        assert!(removed.is_empty());
    }

    #[test]
    fn test_client_disconnect_removes_tools() {
        let r = new_client_tool_registry();
        let c = core_names();
        let tools_arr = [tool_reg("weather", "W"), tool_reg("translate", "T")];
        reg(&r, &c, 1, &tools_arr);
        let removed = handle_client_disconnect(1, &r);
        assert_eq!(removed.len(), 2);
        assert!(!r.lock().unwrap().contains_key(&1));
    }

    #[test]
    fn test_client_disconnect_cancels_pending() {
        let r = new_client_tool_registry();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            r.lock()
                .unwrap()
                .entry(1)
                .or_default()
                .pending_results
                .insert("call-1".into(), tx);
        }
        handle_client_disconnect(1, &r);
        let result = rx.try_recv().unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("disconnected"));
    }

    #[test]
    fn test_handle_tool_result_delivers() {
        let r = new_client_tool_registry();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        {
            r.lock()
                .unwrap()
                .entry(1)
                .or_default()
                .pending_results
                .insert("call-1".into(), tx);
        }
        handle_tool_result(ToolResultArgs {
            client_id: 1,
            tool_call_id: "call-1",
            content: "22°C, sunny",
            is_error: false,
            registry: &r,
        });
        let result = rx.try_recv().unwrap();
        assert_eq!(result.content, "22°C, sunny");
        assert!(!result.is_error);
    }

    #[test]
    fn test_handle_tool_result_unknown_call_id_is_noop() {
        let r = new_client_tool_registry();
        handle_tool_result(ToolResultArgs {
            client_id: 1,
            tool_call_id: "nonexistent",
            content: "data",
            is_error: false,
            registry: &r,
        });
    }

    #[test]
    fn test_two_clients_different_tools() {
        let r = new_client_tool_registry();
        let c = core_names();
        reg(&r, &c, 1, &[tool_reg("weather", "W")]);
        reg(&r, &c, 2, &[tool_reg("translate", "T")]);
        let locked = r.lock().unwrap();
        assert!(locked[&1].tool_names.contains("weather"));
        assert!(locked[&2].tool_names.contains("translate"));
    }
}
