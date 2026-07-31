//! Dispatch-context helpers for UDS extension tool registration/execution.
//!
//! Kept in a sibling module so `uds_ext_protocol.rs` stays under the line-count gate.

use super::*;
// ─── Dispatch helpers (called from uds.rs dispatch_command) ───────────────

/// Handle `register_tools` command in dispatch context.
pub(in crate::interface::cli) async fn dispatch_register_tools(
    ctx: &mut crate::interface::cli::uds::DispatchCtx<'_>,
    id: Option<&str>,
    tools: &[ToolRegistration],
) {
    let ext_names = ctx.agent.tool_registry_extension_names();
    let core_names: std::collections::HashSet<String> = ctx
        .agent
        .tool_descriptors()
        .iter()
        .filter(|d| !ext_names.contains(&d.name().to_string()))
        .map(|d| d.name().to_string())
        .collect();

    let (ok, ev, new_tools) = handle_register_tools(RegisterToolsArgs {
        client_id: ctx.current_client_id,
        id,
        tools,
        registry: &ctx.client_tool_registry,
        core_tool_names: &core_names,
    });
    if !ok || new_tools.is_empty() {
        crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &ev).await;
        return;
    }

    let mut accepted = Vec::new();
    let owner: std::borrow::Cow<'static, str> =
        std::borrow::Cow::Owned(format!("uds:client:{}", ctx.current_client_id));
    for (tool_reg, tool) in tools.iter().zip(new_tools.iter()) {
        if ctx
            .agent
            .register_uds_extension_tool_for_owner(tool.clone(), owner.clone())
        {
            accepted.push(tool_reg.name.clone());
        }
    }

    if accepted.len() != new_tools.len() {
        for name in &accepted {
            ctx.agent.unregister_extension_tool(name);
        }
        let requested: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
        let (_, staged) = handle_unregister_tools(
            ctx.current_client_id,
            None,
            &requested,
            &ctx.client_tool_registry,
        );
        let rejected = requested
            .into_iter()
            .find(|name| !accepted.contains(name))
            .unwrap_or_else(|| "unknown".to_string());
        tracing::warn!(tool = %rejected, ?staged, "UDS tool registration rejected by registry");
        let err = AgentEvent::err(
            id,
            "register_tools",
            format!("tool '{}' was rejected by the tool registry", rejected),
        );
        crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &err).await;
        return;
    }

    let registered_ok = AgentEvent::ok(
        id,
        "register_tools",
        Some(serde_json::json!({ "registered": accepted })),
    );
    crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &registered_ok).await;
    // Spawn a forwarder task for each newly-registered tool. These
    // drain the mpsc receiver stored in `tool_request_rxs` and are
    // the reason tool calls from the LLM actually reach the
    // extension client as `execute_tool` events.
    for tool_reg in tools {
        spawn_tool_forwarder_for(ctx, &tool_reg.name);
    }
    let ext_names = ctx.agent.tool_registry_extension_names();
    let changed = build_extensions_changed_event(&ext_names, ctx.agent);
    crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &changed).await;
}

pub(crate) fn spawn_tool_forwarder_for(
    ctx: &mut crate::interface::cli::uds::DispatchCtx<'_>,
    tool_name: &str,
) {
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
        super::resolve_pending_for_tool(state, tool_name, "Tool re-registered");
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
pub(crate) async fn forward_tool_requests(
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
pub(crate) async fn handle_one_request(
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
                state.insert_pending(
                    tool_call_id.clone(),
                    tool_name.to_string(),
                    reply,
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

    let ev = crate::interface::cli::protocol::AgentEvent::ExecuteTool {
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

/// Handle `unregister_tools` command in dispatch context.
pub(in crate::interface::cli) async fn dispatch_unregister_tools(
    ctx: &mut crate::interface::cli::uds::DispatchCtx<'_>,
    id: Option<&str>,
    tool_names: &[String],
) {
    let (ev, removed) = handle_unregister_tools(
        ctx.current_client_id,
        id,
        tool_names,
        &ctx.client_tool_registry,
    );
    crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &ev).await;

    if !removed.is_empty() {
        for name in &removed {
            ctx.agent.unregister_extension_tool(name);
        }
        let ext_names = ctx.agent.tool_registry_extension_names();
        let changed = build_extensions_changed_event(&ext_names, ctx.agent);
        crate::interface::cli::uds::emit_event_to_broadcast_or_writer(ctx, &changed).await;
    }
}

/// Handle `tool_result` command in dispatch context.
pub(in crate::interface::cli) fn dispatch_tool_result(
    ctx: &mut crate::interface::cli::uds::DispatchCtx<'_>,
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
