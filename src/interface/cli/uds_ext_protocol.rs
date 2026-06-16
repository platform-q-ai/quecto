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

use crate::domain::extension_tool::ToolInvocation;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::extensions::uds_tool::create_uds_tool;

use super::protocol::{AgentEvent, ExtensionInfo, ToolRegistration};

/// Default timeout for UDS extension tool execution (seconds).
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 30;

/// Handle to a spawned forwarder task. Stored alongside a
/// cooperative shutdown `oneshot` so unregister / disconnect can ask
/// the task to drain its inbound mpsc of any buffered
/// `ToolInvocation`s — resolving their oneshots with an immediate
/// error — instead of abort-killing the task and leaving those
/// in-flight callers to wait out the 30-second UdsTool timeout.
#[derive(Debug)]
pub struct ForwarderHandle {
    /// JoinHandle is kept so `Drop` detaches cleanly; we never
    /// `.abort()` it — a voluntary shutdown is always preferred so the
    /// drain runs.
    join_handle: tokio::task::JoinHandle<()>,
    /// One-shot signal: send a short reason string and the task
    /// shifts into drain-and-exit mode.
    shutdown: Option<tokio::sync::oneshot::Sender<&'static str>>,
}

impl ForwarderHandle {
    /// Signal a graceful shutdown with `reason`. The task drains any
    /// buffered requests, resolves their oneshots with an error
    /// carrying `reason`, then exits. Consumes `self`.
    pub fn shutdown(mut self, reason: &'static str) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(reason);
        }
        // Drop self — JoinHandle detaches, task completes on its own.
        std::mem::drop(self.join_handle);
    }
}

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
    pub tool_request_rxs: HashMap<String, tokio::sync::mpsc::Receiver<ToolInvocation>>,
    /// Forwarder tasks that consume `ToolInvocation`s from the mpsc
    /// receiver created by `create_uds_tool`, register the oneshot
    /// result sender into `pending_results`, and emit an `execute_tool`
    /// event to the wire. Keyed by tool name. On unregister/disconnect
    /// we signal `shutdown` rather than `abort` so buffered requests
    /// are drained with a clean error.
    pub tool_request_tasks: HashMap<String, ForwarderHandle>,
    /// Per-client *targeted* event channel (V4). The forwarder uses
    /// this — not the shared broadcast — to deliver `execute_tool` to
    /// the specific client that registered the tool.  Other
    /// connected clients (e.g. a separate `quecto-tui` watching the
    /// same agent) never see the tool name or arguments of requests
    /// that aren't addressed to them.  Set on accept via
    /// `register_client_writer`; cleared on disconnect.
    pub writer_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

/// Register a per-client writer sender so `forward_tool_requests` can
/// route `execute_tool` events to this client only.  Called from the
/// accept loop the moment a new client connection is set up, before
/// any `register_tools` command can arrive.
pub fn register_client_writer(
    registry: &ClientToolRegistry,
    client_id: u64,
    writer_tx: tokio::sync::mpsc::Sender<String>,
) {
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    let state = reg.entry(client_id).or_default();
    state.writer_tx = Some(writer_tx);
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

    let mut seen = HashSet::new();
    for tool in tools {
        if !seen.insert(tool.name.as_str()) {
            let ev = AgentEvent::err(
                id,
                "register_tools",
                format!(
                    "tool '{}' is registered more than once in this request",
                    tool.name
                ),
            );
            return (false, ev, vec![]);
        }
    }

    let timeout = std::time::Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS);
    let mut new_tools: Vec<Arc<dyn crate::domain::tool::Tool>> = Vec::new();
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());

    for tool in tools {
        if let Some((owner, _)) = reg.iter().find(|(owner_id, state)| {
            **owner_id != client_id && state.tool_names.contains(&tool.name)
        }) {
            let ev = AgentEvent::err(
                id,
                "register_tools",
                format!(
                    "tool '{}' is already registered by client {}",
                    tool.name, owner
                ),
            );
            return (false, ev, vec![]);
        }
    }

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
                    // Graceful shutdown: task drains any buffered
                    // ToolInvocations and errors them with this reason
                    // (clients see an immediate result rather than
                    // waiting out the UdsTool timeout).
                    handle.shutdown("Tool unregistered");
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

    // Graceful shutdown of each forwarder: drain any buffered
    // requests and reply with "Extension disconnected" so in-flight
    // callers get an immediate error. The tasks finish on their own
    // after the drain — no .abort() needed.
    for (_, handle) in state.tool_request_tasks {
        handle.shutdown("Extension disconnected");
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
/// record the task handle for shutdown on unregister/disconnect.
fn spawn_tool_forwarder_for(ctx: &mut super::uds::DispatchCtx<'_>, tool_name: &str) {
    let client_id = ctx.current_client_id;
    let registry = ctx.client_tool_registry.clone();

    // Snapshot the rx (to own) and writer_tx (to clone) under a
    // single short critical section.
    let staged = {
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        let state = match reg.get_mut(&client_id) {
            Some(s) => s,
            None => return,
        };
        if let Some(old) = state.tool_request_tasks.remove(tool_name) {
            old.shutdown("Tool re-registered");
        }
        let rx = state.tool_request_rxs.remove(tool_name);
        let writer_tx = state.writer_tx.clone();
        rx.map(|rx| (rx, writer_tx))
    };
    let Some((rx, writer_tx)) = staged else {
        return;
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let name_for_task = tool_name.to_string();
    let join_handle = tokio::spawn(forward_tool_requests(
        client_id,
        name_for_task,
        rx,
        shutdown_rx,
        registry.clone(),
        writer_tx,
    ));

    let handle = ForwarderHandle {
        join_handle,
        shutdown: Some(shutdown_tx),
    };

    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = reg.get_mut(&client_id) {
        state
            .tool_request_tasks
            .insert(tool_name.to_string(), handle);
    } else {
        // Client disappeared between the two locks. Signal shutdown
        // so the task drains cleanly instead of hanging.
        handle.shutdown("Client gone");
    }
}

/// Drain `ToolInvocation`s for a single (client, tool) pair: stash the
/// oneshot result sender in `pending_results` so `tool_result` handlers
/// can resolve it, then emit an `execute_tool` event to the wire,
/// addressed via the client's per-connection `writer_tx`.  Scoping to
/// one client prevents other connected clients from ever seeing tool
/// names or arguments not addressed to them.
async fn forward_tool_requests(
    client_id: u64,
    tool_name: String,
    mut rx: tokio::sync::mpsc::Receiver<ToolInvocation>,
    mut shutdown: tokio::sync::oneshot::Receiver<&'static str>,
    registry: ClientToolRegistry,
    writer_tx: Option<tokio::sync::mpsc::Sender<String>>,
) {
    // Hot loop: serve requests or honour a shutdown signal. `biased`
    // so a shutdown that arrives concurrently with a buffered request
    // preempts normal dispatch — once unregister has been requested
    // we don't want any more execute_tool events going out, only
    // drain-with-error for whatever is already queued.
    let shutdown_reason: Option<&'static str> = loop {
        tokio::select! {
            biased;
            reason = &mut shutdown => {
                break reason.ok();
            }
            maybe_req = rx.recv() => {
                let Some(req) = maybe_req else { break None; };
                handle_one_request(&tool_name, req, client_id, &registry, &writer_tx).await;
            }
        }
    };

    // Drain: any request already in the mpsc queue gets an immediate
    // error instead of sitting forever (would be 30s timeout from
    // UdsTool::execute). The drain is bounded by the mpsc buffer size.
    let drain_reason = shutdown_reason.unwrap_or("Extension disconnected");
    rx.close();
    while let Ok(req) = rx.try_recv() {
        let _ = req.reply.send(ToolResult {
            content: drain_reason.to_string(),
            is_error: true,
            image_blocks: vec![],
        });
    }
}

/// Single-request side of `forward_tool_requests`: stash the oneshot,
/// send `execute_tool` to the target client's writer channel, clean
/// up on delivery failure.
async fn handle_one_request(
    tool_name: &str,
    req: ToolInvocation,
    client_id: u64,
    registry: &ClientToolRegistry,
    writer_tx: &Option<tokio::sync::mpsc::Sender<String>>,
) {
    let ToolInvocation {
        tool_call_id,
        tool_name: sent_tool,
        arguments,
        reply,
    } = req;

    // Dispatch bugs would show up here as a mismatch between the tool
    // the UdsTool thinks it's calling and the one we're forwarding
    // for. Catch it loudly in debug builds; in release we use the
    // forwarder's own `tool_name` (the one keyed into this task at
    // registration).
    debug_assert_eq!(
        sent_tool, tool_name,
        "forwarder tool_name mismatch: registered={tool_name:?} request={sent_tool:?}"
    );

    // Stash the oneshot in `pending_results` BEFORE sending
    // `execute_tool` so the reader task's `handle_tool_result` (which
    // takes the same registry mutex) always finds the pending entry
    // when the client responds — even on the fastest possible local-
    // socket round-trip.
    {
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        match reg.get_mut(&client_id) {
            Some(state) => {
                state.pending_results.insert(tool_call_id.clone(), reply);
                spawn_pending_timeout_cleanup(
                    client_id,
                    tool_call_id.clone(),
                    registry.clone(),
                    std::time::Duration::from_secs(DEFAULT_TOOL_TIMEOUT_SECS),
                );
            }
            None => {
                // Client gone — drop the request; the UdsTool's
                // `result_rx` then resolves with `Err(RecvError)` and
                // surfaces "Extension disconnected during execution"
                // upstream.
                return;
            }
        }
    }

    let ev = super::protocol::AgentEvent::ExecuteTool {
        tool_call_id: tool_call_id.clone(),
        tool_name: tool_name.to_string(),
        arguments,
    };

    // Deliver to the registering client's targeted writer channel.
    // If we have no writer (e.g. client disconnect window, or a test
    // harness that set up the state without one) or the send fails
    // (receiver dropped), proactively remove the pending entry so
    // `UdsTool::execute` fails fast with "Extension disconnected"
    // rather than waiting out the 30-second tool timeout.
    let delivered = match writer_tx {
        Some(tx) => {
            let mut line = ev.to_json_line();
            line.push('\n');
            tx.send(line).await.is_ok()
        }
        None => false,
    };
    if !delivered {
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = reg.get_mut(&client_id) {
            state.pending_results.remove(&tool_call_id);
        }
    }
}

fn spawn_pending_timeout_cleanup(
    client_id: u64,
    tool_call_id: String,
    registry: ClientToolRegistry,
    timeout: std::time::Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = reg.get_mut(&client_id) {
            state.pending_results.remove(&tool_call_id);
        }
    });
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
    fn test_register_tools_rejects_duplicate_owner() {
        let r = new_client_tool_registry();
        let c = core_names();
        let (ok, _, _) = reg(&r, &c, 1, &[tool_reg("weather", "Owned by one")]);
        assert!(ok);

        let (ok, ev, tools) = reg(&r, &c, 2, &[tool_reg("weather", "Hijack")]);
        assert!(!ok);
        assert!(tools.is_empty());
        let json = ev.to_json_line();
        assert!(json.contains("already registered by client"));
        assert!(r.lock().unwrap()[&1].tool_names.contains("weather"));
        assert!(!r.lock().unwrap().contains_key(&2));
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

    // ─── forward_tool_requests cleanup-on-delivery-failure (V3) ────────────

    /// When the targeted writer channel's receiver has been dropped
    /// (client disconnected, test harness never connected one), every
    /// `send` returns `Err(SendError)`. Without cleanup the oneshot
    /// sender would sit in `pending_results` until the 30-second
    /// UdsTool timeout fired — forever, for repeated calls into a
    /// dead writer. The forwarder must remove the pending entry so
    /// the oneshot drops and UdsTool::execute returns "Extension
    /// disconnected during execution" immediately.
    #[tokio::test]
    async fn pending_timeout_cleanup_removes_stale_entry() {
        let registry = new_client_tool_registry();
        let client_id = 55;
        let (reply_tx, _reply_rx) = tokio::sync::oneshot::channel();
        {
            let mut reg = registry.lock().unwrap();
            let state = reg.entry(client_id).or_default();
            state.pending_results.insert("stale-call".into(), reply_tx);
        }

        spawn_pending_timeout_cleanup(
            client_id,
            "stale-call".into(),
            registry.clone(),
            std::time::Duration::from_millis(10),
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let reg = registry.lock().unwrap();
        assert!(reg[&client_id].pending_results.is_empty());
    }

    #[tokio::test]
    async fn forwarder_cleans_pending_when_writer_has_no_receiver() {
        use crate::domain::extension_tool::ToolInvocation;
        use std::time::Duration;

        let registry = new_client_tool_registry();
        let client_id: u64 = 42;
        {
            let mut reg = registry.lock().unwrap();
            reg.insert(client_id, ClientToolState::default());
        }

        // Writer channel with no receiver — every .send() errs.
        let (writer_tx, writer_rx) = tokio::sync::mpsc::channel::<String>(8);
        drop(writer_rx);

        let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(4);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();
        let forwarder = tokio::spawn(super::forward_tool_requests(
            client_id,
            "weather".to_string(),
            req_rx,
            shutdown_rx,
            registry.clone(),
            Some(writer_tx),
        ));

        let (reply_tx, result_rx) = tokio::sync::oneshot::channel();
        req_tx
            .send(ToolInvocation {
                tool_call_id: "uds-test-1".into(),
                tool_name: "weather".into(),
                arguments: "{}".into(),
                reply: reply_tx,
            })
            .await
            .unwrap();

        // Oneshot must resolve (closed by cleanup dropping the sender)
        // within a tick — NOT wait the full 30s default tool timeout.
        let awaited = tokio::time::timeout(Duration::from_millis(500), result_rx).await;
        assert!(
            awaited.is_ok(),
            "forwarder failed to resolve oneshot within 500ms — pending leak?"
        );
        let recv_result = awaited.unwrap();
        assert!(
            recv_result.is_err(),
            "sender should have been dropped, leading to Err on the receiver"
        );

        // Pending table for this client is empty again.
        let reg = registry.lock().unwrap();
        let state = reg.get(&client_id).expect("client state");
        assert!(
            state.pending_results.is_empty(),
            "pending_results leaked after failed broadcast: {:?}",
            state.pending_results.keys().collect::<Vec<_>>()
        );

        drop(req_tx);
        let _ = forwarder.await;
    }

    /// N3 — When a client unregisters a tool (or disconnects) while a
    /// ToolInvocation is already buffered in the mpsc but not yet
    /// consumed by the forwarder, the request must resolve promptly
    /// with an error carrying the shutdown reason — not sit there
    /// waiting out the 30-second UdsTool timeout.
    #[tokio::test]
    async fn forwarder_drains_buffered_requests_on_shutdown() {
        use crate::domain::extension_tool::ToolInvocation;
        use std::time::Duration;

        let registry = new_client_tool_registry();
        let client_id: u64 = 101;
        {
            let mut reg = registry.lock().unwrap();
            reg.insert(client_id, ClientToolState::default());
        }

        let (writer_tx, _writer_rx) = tokio::sync::mpsc::channel::<String>(8);
        let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(8);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();

        // Queue the requests AND fire shutdown BEFORE spawning the
        // forwarder. When the task starts, `biased; shutdown` wins
        // deterministically and the subsequent drain pass picks up
        // every already-buffered request.
        let (r1_tx, r1_rx) = tokio::sync::oneshot::channel();
        let (r2_tx, r2_rx) = tokio::sync::oneshot::channel();
        req_tx
            .send(ToolInvocation {
                tool_call_id: "drain-1".into(),
                tool_name: "weather".into(),
                arguments: "{}".into(),
                reply: r1_tx,
            })
            .await
            .unwrap();
        req_tx
            .send(ToolInvocation {
                tool_call_id: "drain-2".into(),
                tool_name: "weather".into(),
                arguments: "{}".into(),
                reply: r2_tx,
            })
            .await
            .unwrap();
        shutdown_tx.send("Tool unregistered").unwrap();

        // Drop our sender so the forwarder's drain loop eventually
        // sees an empty channel. (It already exits after the shutdown
        // branch fires, but closing the channel is defensive.)
        drop(req_tx);

        let forwarder = tokio::spawn(super::forward_tool_requests(
            client_id,
            "weather".to_string(),
            req_rx,
            shutdown_rx,
            registry.clone(),
            Some(writer_tx),
        ));

        let r1 = tokio::time::timeout(Duration::from_millis(500), r1_rx)
            .await
            .expect("buffered request 1 hung past 500ms")
            .expect("result_tx dropped without send");
        let r2 = tokio::time::timeout(Duration::from_millis(500), r2_rx)
            .await
            .expect("buffered request 2 hung past 500ms")
            .expect("result_tx dropped without send");

        assert!(r1.is_error);
        assert!(r1.content.contains("unregistered"));
        assert!(r2.is_error);
        assert!(r2.content.contains("unregistered"));

        let _ = forwarder.await;
    }

    /// Control case: when the targeted writer channel DOES have a
    /// live receiver the forwarder leaves the pending entry in place
    /// (the reader task will clear it when `tool_result` comes back).
    #[tokio::test]
    async fn forwarder_leaves_pending_when_writer_delivered() {
        use crate::domain::extension_tool::ToolInvocation;
        use std::time::Duration;

        let registry = new_client_tool_registry();
        let client_id: u64 = 7;
        {
            let mut reg = registry.lock().unwrap();
            reg.insert(client_id, ClientToolState::default());
        }

        let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(8);

        let (req_tx, req_rx) = tokio::sync::mpsc::channel::<ToolInvocation>(4);
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<&'static str>();
        let forwarder = tokio::spawn(super::forward_tool_requests(
            client_id,
            "weather".to_string(),
            req_rx,
            shutdown_rx,
            registry.clone(),
            Some(writer_tx),
        ));

        let (reply_tx, _result_rx) = tokio::sync::oneshot::channel();
        req_tx
            .send(ToolInvocation {
                tool_call_id: "uds-test-2".into(),
                tool_name: "weather".into(),
                arguments: "{}".into(),
                reply: reply_tx,
            })
            .await
            .unwrap();

        // Receive the targeted event (proving delivery happened).
        let line = tokio::time::timeout(Duration::from_millis(500), writer_rx.recv())
            .await
            .expect("writer event never arrived")
            .expect("writer channel closed unexpectedly");
        assert!(line.contains("\"execute_tool\""));

        // Short yield so the forwarder-side insert completes even if
        // the send path beat it to the broadcast receiver.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let reg = registry.lock().unwrap();
        let state = reg.get(&client_id).expect("client state");
        assert_eq!(
            state.pending_results.len(),
            1,
            "pending entry should remain until tool_result arrives"
        );

        drop(req_tx);
        drop(reg);
        let _ = forwarder.await;
    }
}
