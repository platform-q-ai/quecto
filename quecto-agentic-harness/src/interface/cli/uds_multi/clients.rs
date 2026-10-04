//! The dispatch loop's client bookkeeping: a client's disconnect (its tools
//! leave with it), and the agent's configured extensions (#2446), which are
//! its own connections rather than clients: until they have registered
//! their tools, or the wait ran out, only they are served and everything
//! else is held, in order, so the first turn already has their tools; they
//! neither keep the agent alive nor end it by leaving. Child module of
//! `uds_multi`.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};

use super::recv::{DispatchMsg, recv_next_message};
use super::{
    AgentEvent, ClientMessage, DispatchCtx, DispatchLoopArgs, emit_event_to_broadcast_or_writer,
    handle_client_msg,
};
use crate::domain::agents::configured_extensions::REGISTRATION_WAIT;
use crate::interface::cli::uds_extensions::{Extensions, extension_clients, extension_of};

/// Serve only the configured extensions' own connections until they have
/// settled; return what arrived from everyone else meanwhile, in order (a
/// shutdown first).
pub(super) async fn hold_for_extensions(
    ctx: &mut DispatchCtx<'_>,
    args: &mut DispatchLoopArgs,
    live_clients: &AtomicU32,
) -> VecDeque<DispatchMsg> {
    let mut held = VecDeque::new();
    let Some(extensions) = args.extensions.clone() else {
        return held;
    };
    ctx.session.observe_extensions(Some(extensions.clone()));
    let settled = tokio::time::timeout(REGISTRATION_WAIT, extensions.settled());
    tokio::pin!(settled);
    loop {
        let next = recv_next_message(
            &mut args.cmd_rx,
            &mut args.disconnect_rx,
            &mut ctx.notification_rx,
            &args.shutdown,
        );
        let msg = tokio::select! {
            biased;
            _ = &mut settled => break,
            msg = next => msg,
        };
        match msg {
            Some(DispatchMsg::Client(ClientMessage::Command(cmd)))
                if extension_of(&ctx.client_tool_registry, cmd.client_id).is_some() =>
            {
                let command = ClientMessage::Command(cmd);
                let _ = handle_client_msg(ctx, command, args.lifetime, live_clients).await;
                super::refresh_tool_catalogue_snapshot(ctx).await;
                super::refresh_state_snapshot(ctx).await;
            }
            Some(DispatchMsg::Shutdown) => {
                held.push_front(DispatchMsg::Shutdown);
                break;
            }
            Some(other) => held.push_back(other),
            None => break,
        }
    }
    held
}

/// The next message: a held one first, then whatever arrives.
pub(super) async fn next(
    held: &mut VecDeque<DispatchMsg>,
    ctx: &mut DispatchCtx<'_>,
    args: &mut DispatchLoopArgs,
) -> Option<DispatchMsg> {
    match held.pop_front() {
        Some(msg) => Some(msg),
        None => {
            recv_next_message(
                &mut args.cmd_rx,
                &mut args.disconnect_rx,
                &mut ctx.notification_rx,
                &args.shutdown,
            )
            .await
        }
    }
}

/// End the agent's configured extensions as it exits.
pub(super) async fn shut_down(extensions: &Extensions) {
    if let Some(extensions) = extensions {
        extensions.shutdown().await;
    }
}

/// Client `client_id` disconnected: its tools leave. Whether the agent
/// exits with it: the last client of a lifetime that ends with it, the
/// agent's own extensions not counted.
pub(super) async fn disconnected(
    ctx: &mut DispatchCtx<'_>,
    client_id: u64,
    lifetime: crate::domain::harness_lifetime::HarnessLifetime,
    live_clients: &AtomicU32,
) -> bool {
    let client = extension_of(&ctx.client_tool_registry, client_id).is_none();
    handle_disconnect(ctx, client_id).await;
    let owned = extension_clients(&ctx.client_tool_registry);
    client
        && lifetime.exits_when_last_client_disconnects()
        && live_clients.load(Ordering::SeqCst).saturating_sub(owned) == 0
}

/// Unregister tools owned by a disconnecting client (#352).
async fn handle_disconnect(ctx: &mut DispatchCtx<'_>, client_id: u64) {
    let before: Vec<serde_json::Value> = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .map(|entry| serde_json::to_value(entry).unwrap_or_default())
        .collect();
    let removed = crate::interface::cli::uds_ext_protocol::handle_client_disconnect(
        client_id,
        &ctx.client_tool_registry,
    );
    if !removed.is_empty() {
        ctx.agent.unregister_uds_tools_for_client(client_id);
        // #2446: children spawned from now on inherit the current catalogue.
        ctx.agent.refresh_spawn_inherited_child_policy_snapshot();
        let after: Vec<serde_json::Value> = ctx
            .agent
            .tool_catalogue_entries()
            .into_iter()
            .map(|entry| serde_json::to_value(entry).unwrap_or_default())
            .collect();
        {
            let mut snapshot = ctx.tool_catalogue_snapshot.write().await;
            *snapshot = after.clone();
        }
        let ev = AgentEvent::ToolCatalogueChanged {
            changed_tools: removed,
            before,
            after,
            reason: "client_disconnect".to_string(),
        };
        emit_event_to_broadcast_or_writer(ctx, &ev).await;
    }
}
