//! The dispatch loop's client bookkeeping: a client's disconnect (its tools
//! leave with it), and the agent's configured extensions (#2446), which are
//! its own connections rather than clients: until they have registered
//! their tools, or the wait ran out, work that starts a turn is held, in
//! order, so the first turn already has their tools (queries, tool
//! registrations and disconnects are served as ever); they neither keep the
//! agent alive nor end it by leaving. Child module of `uds_multi`.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};

use super::recv::{DispatchMsg, recv_next_message};
use super::{
    AgentEvent, ClientMessage, DispatchCtx, DispatchLoopArgs, emit_event_to_broadcast_or_writer,
};
use crate::domain::agents::configured_extensions::REGISTRATION_WAIT;
use crate::interface::cli::uds_extensions::{Extensions, extension_clients, extension_of};

/// The start-up wait for the configured extensions: until they have
/// registered their tools (or [`REGISTRATION_WAIT`] passed), work that
/// starts a turn is held, in order; everything else is served at once.
pub(super) struct StartupHold {
    wait: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    held: VecDeque<DispatchMsg>,
}

impl StartupHold {
    pub(super) fn new(ctx: &mut DispatchCtx<'_>, extensions: Extensions) -> Self {
        let wait = extensions.map(|extensions| {
            ctx.session.observe_extensions(Some(extensions.clone()));
            Box::pin(async move {
                let settled = tokio::time::timeout(REGISTRATION_WAIT, extensions.settled()).await;
                if settled.is_err() {
                    tracing::warn!("configured extensions still unsettled; taking work");
                }
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        });
        Self {
            wait,
            held: VecDeque::new(),
        }
    }

    /// The next message: a held one once the wait is over, else whatever
    /// arrives.
    pub(super) async fn next(
        &mut self,
        ctx: &mut DispatchCtx<'_>,
        args: &mut DispatchLoopArgs,
    ) -> Option<DispatchMsg> {
        loop {
            let Some(wait) = self.wait.as_mut() else {
                if let Some(msg) = self.held.pop_front() {
                    return Some(msg);
                }
                return recv_next_message(
                    &mut args.cmd_rx,
                    &mut args.disconnect_rx,
                    &mut ctx.notification_rx,
                    &args.shutdown,
                )
                .await;
            };
            tokio::select! {
                biased;
                () = wait => {}
                msg = recv_next_message(
                    &mut args.cmd_rx,
                    &mut args.disconnect_rx,
                    &mut ctx.notification_rx,
                    &args.shutdown,
                ) => return msg,
            }
            self.wait = None;
        }
    }

    /// `msg` to handle now, or `None` when it starts a turn during the wait
    /// and is held.
    pub(super) fn admit(&mut self, msg: DispatchMsg) -> Option<DispatchMsg> {
        match (self.wait.is_some(), starts_a_turn(&msg)) {
            (true, true) => {
                self.held.push_back(msg);
                None
            }
            (true, false) | (false, _) => Some(msg),
        }
    }
}

/// Whether `msg` starts a turn: a prompt, steer or follow-up, a swarm wake,
/// a sub-agent's note.
fn starts_a_turn(msg: &DispatchMsg) -> bool {
    match msg {
        DispatchMsg::Client(ClientMessage::Command(command)) => {
            let kind = serde_json::from_str::<serde_json::Value>(&command.line)
                .ok()
                .and_then(|line| line["type"].as_str().map(str::to_owned));
            matches!(kind.as_deref(), Some("prompt" | "steer" | "follow_up"))
        }
        DispatchMsg::Client(ClientMessage::SwarmWake { .. }) | DispatchMsg::Notification(_) => true,
        DispatchMsg::Client(ClientMessage::Disconnected(_) | ClientMessage::RejectedControl(_))
        | DispatchMsg::Shutdown => false,
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
