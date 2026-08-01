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

use super::protocol::{AgentEvent, ToolRegistration};

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

/// A parked tool-result sender together with the instant past which the entry
/// is considered stale and may be swept.
#[derive(Debug)]
pub struct PendingResult {
    pub reply: tokio::sync::oneshot::Sender<ToolResult>,
    pub deadline: std::time::Instant,
    pub tool_name: String,
}

impl ClientToolState {
    /// Park a pending result sender under `tool_call_id` with an expiry
    /// `timeout` from now, first sweeping any entries whose deadline has already
    /// passed. Sweeping lazily on insert replaces the previous per-call spawned
    /// timer task (#996): the caller's `UdsTool::execute` timeout already
    /// unblocks it, so dropping stale senders here only reclaims map slots.
    pub fn insert_pending(
        &mut self,
        tool_call_id: String,
        tool_name: String,
        reply: tokio::sync::oneshot::Sender<ToolResult>,
        timeout: std::time::Duration,
    ) {
        let deadline = std::time::Instant::now() + timeout;
        self.sweep_expired_pending();
        self.pending_results.insert(
            tool_call_id,
            PendingResult {
                reply,
                deadline,
                tool_name,
            },
        );
    }

    /// Drop pending entries whose deadline has passed. Called from every path
    /// that already touches the map (`insert_pending`, `handle_tool_result`) so
    /// a timed-out call on an otherwise idle client is reclaimed by the
    /// client's own late `tool_result`, not only by the next insert.
    pub fn sweep_expired_pending(&mut self) {
        let now = std::time::Instant::now();
        self.pending_results
            .retain(|_, pending| pending.deadline > now);
    }
}

/// Per-client tool state.
#[derive(Debug, Default)]
pub struct ClientToolState {
    /// Tool names registered by this client.
    pub tool_names: HashSet<String>,
    /// Pending tool execution results: tool_call_id → result sender + deadline.
    /// The forwarder task below populates this when it dispatches an
    /// `execute_tool`; `handle_tool_result` drains it when the client's
    /// `tool_result` arrives. Expired entries are swept lazily on the next
    /// insert (see `insert_pending`) — the caller's own `UdsTool::execute`
    /// timeout already unblocks it, so this only reclaims the map slot, without
    /// a spawned 30s timer task per tool call (#996).
    pub pending_results: HashMap<String, PendingResult>,
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

        // If tool was already registered by this client, replace the old
        // generation: drop staged rx, shut down any live forwarder, and
        // immediately fail already-dispatched pending calls for this tool.
        if state.tool_names.contains(&tool_reg.name) {
            state.tool_request_rxs.remove(&tool_reg.name);
            if let Some(old) = state.tool_request_tasks.remove(&tool_reg.name) {
                old.shutdown("Tool re-registered");
            }
            resolve_pending_for_tool(state, &tool_reg.name, "Tool re-registered");
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
                // Also fail already-dispatched pending oneshots for this
                // tool so late tool_result cannot complete after unregister
                // and callers do not wait out the full timeout.
                resolve_pending_for_tool(state, name, "Tool unregistered");
                removed.push(name.clone());
            }
        }
        // Drop empty client entries so a failed registration rollback does
        // not leave a zombie ownership record behind.
        if state.tool_names.is_empty()
            && state.tool_request_rxs.is_empty()
            && state.tool_request_tasks.is_empty()
            && state.pending_results.is_empty()
        {
            reg.remove(&client_id);
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
        if let Some(pending) = state.pending_results.remove(tool_call_id) {
            let _ = pending.reply.send(ToolResult {
                content: content.to_string(),
                is_error,
                image_blocks: vec![],
            });
        }
        // Reclaim any other entries whose caller has already timed out — e.g.
        // this very result arriving late for a call `UdsTool::execute` gave up
        // on — so an idle client doesn't hold stale slots until its next call.
        state.sweep_expired_pending();
    }
}

/// Clone the per-client *targeted* writer sender, if registered (#876).
///
/// Lets the per-connection reader task push a line (e.g. an early acceptance
/// ack for a forwarded control command) directly to THIS client's socket via
/// its `writer_tx`/`targeted_rx` pair, bypassing the single shared dispatch
/// loop — which may be blocked mid-turn. The lock is held only for the clone.
pub fn client_writer_tx(
    registry: &ClientToolRegistry,
    client_id: u64,
) -> Option<tokio::sync::mpsc::Sender<String>> {
    let reg = registry.lock().unwrap_or_else(|e| e.into_inner());
    reg.get(&client_id).and_then(|s| s.writer_tx.clone())
}

/// Deliver an intercepted control forward's acceptance ack (#876) to THIS
/// client's serialized writer channel — bypassing the possibly-blocked dispatch
/// loop — and return the transformed work line for the caller to enqueue, if
/// any. Extracted from the reader loop so the ack→writer-channel wiring is
/// unit-testable without standing up a full server.
pub(super) async fn ack_accepted_control(
    registry: &ClientToolRegistry,
    client_id: u64,
    ctrl: super::uds_control_forward::AcceptedControl,
) -> Option<String> {
    if let Some(tx) = client_writer_tx(registry, client_id) {
        let _ = tx.send(ctrl.ack_line).await;
    }
    ctrl.forward_line
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
    for (_, pending) in state.pending_results {
        let _ = pending.reply.send(ToolResult {
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

fn resolve_pending_for_tool(state: &mut ClientToolState, tool_name: &str, reason: &str) {
    let pending_ids: Vec<String> = state
        .pending_results
        .keys()
        .filter(|id| state.pending_results[*id].tool_name == tool_name)
        .cloned()
        .collect();
    for id in pending_ids {
        if let Some(pending) = state.pending_results.remove(&id) {
            let _ = pending.reply.send(ToolResult {
                content: reason.to_string(),
                is_error: true,
                image_blocks: vec![],
            });
        }
    }
}

// Re-export dispatch helpers into this module for existing call sites.
#[path = "uds_ext_protocol_dispatch.rs"]
mod dispatch;
pub(in crate::interface::cli) use dispatch::{
    dispatch_register_tools, dispatch_tool_result, dispatch_unregister_tools,
};
#[cfg(test)]
pub(super) use dispatch::{forward_tool_requests, handle_one_request};

#[cfg(test)]
#[path = "uds_ext_protocol_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "uds_ext_protocol_dispatch_cov_tests.rs"]
mod dispatch_cov_tests;

#[cfg(test)]
#[path = "uds_ext_protocol_tests.rs"]
mod tests;
