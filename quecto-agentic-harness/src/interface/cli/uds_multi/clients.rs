//! The dispatch loop's client bookkeeping: a client's disconnect (its tools
//! leave with it), and the agent's configured extensions (#2446), which are
//! its own connections rather than clients: until they have registered
//! their tools, or the wait ran out, only reads and their own traffic are
//! served and everything else is held, in order, so the first turn already
//! has their tools; they neither keep the agent alive nor end it by
//! leaving, and they start ending as soon as a shutdown is admitted. Child module of `uds_multi`.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};

use super::recv::{DispatchMsg, recv_next_message};
use super::{
    AgentEvent, ClientMessage, DispatchCtx, DispatchLoopArgs, emit_event_to_broadcast_or_writer,
};
use crate::domain::agents::configured_extensions::REGISTRATION_WAIT;
use crate::interface::cli::uds_extensions::{Extensions, extension_clients, extension_of};

/// The start-up wait for the configured extensions: until they have
/// registered their tools (or [`REGISTRATION_WAIT`] passed), only reads and
/// the extensions' own traffic are served; everything else is held, in
/// order.
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

    /// The next message to handle: held ones in order once the wait is
    /// over; during it, only what [`served_during_the_wait`] allows.
    pub(super) async fn next(
        &mut self,
        ctx: &mut DispatchCtx<'_>,
        args: &mut DispatchLoopArgs,
    ) -> Option<DispatchMsg> {
        loop {
            let msg = self.receive(ctx, args).await?;
            if let Some(msg) = self.admit(ctx, msg) {
                return Some(msg);
            }
        }
    }

    /// The next message: a held one once the wait is over, else whatever
    /// arrives.
    async fn receive(
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

    /// `msg` to handle now, or `None` when it is held: during the wait only
    /// what [`served_during_the_wait`] allows is handled; everything else
    /// waits, in order (#1720: a client's commands outrank its disconnect).
    fn admit(&mut self, ctx: &DispatchCtx<'_>, msg: DispatchMsg) -> Option<DispatchMsg> {
        if self.wait.is_none() {
            return Some(msg);
        }
        match served_during_the_wait(ctx, &msg) {
            true => Some(msg),
            false => {
                self.held.push_back(msg);
                None
            }
        }
    }
}

/// Commands any client may have served during the start-up wait: reads,
/// and the UDS tool traffic extensions need. An allowlist.
const SERVED_COMMANDS: &[&str] = &[
    "get_state",
    "get_report",
    "get_messages",
    "get_messages_tail",
    "get_message",
    "get_session_stats",
    "get_subagents",
    "get_tool_catalogue",
    "list_tools",
    "list_models",
    "list_sessions",
    "search_session_metadata",
    "sync",
    "register_tools",
    "unregister_tools",
    "tool_result",
];

/// Whether `msg` is served during the wait: a shutdown, a configured
/// extension's own command or disconnect, or a [`SERVED_COMMANDS`] command.
/// Everything else (turns, state changes, other clients' disconnects) waits.
fn served_during_the_wait(ctx: &DispatchCtx<'_>, msg: &DispatchMsg) -> bool {
    let extension = |client_id| extension_of(&ctx.client_tool_registry, client_id).is_some();
    match msg {
        DispatchMsg::Shutdown => true,
        DispatchMsg::Client(ClientMessage::Command(command)) => {
            extension(command.client_id) || {
                let kind = serde_json::from_str::<serde_json::Value>(&command.line)
                    .ok()
                    .and_then(|line| line["type"].as_str().map(str::to_owned));
                kind.is_some_and(|kind| SERVED_COMMANDS.contains(&kind.as_str()))
            }
        }
        DispatchMsg::Client(ClientMessage::Disconnected(disconnected)) => {
            extension(disconnected.client_id)
        }
        DispatchMsg::Client(
            ClientMessage::SwarmWake { .. } | ClientMessage::RejectedControl(_),
        )
        | DispatchMsg::Notification(_) => false,
    }
}

/// Start ending the configured extensions as soon as a shutdown is admitted
/// (the harness lifecycle freezes), alongside the fleet teardown, so a
/// child's exit fits its launcher's budget.
pub(super) fn end_with_the_fleet(
    extensions: &Extensions,
    lifecycle: Option<crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle>,
) {
    use crate::domain::agents::subagent_teardown::HarnessLifecycleState;
    let (Some(extensions), Some(lifecycle)) = (extensions.clone(), lifecycle) else {
        return;
    };
    tokio::spawn(async move {
        loop {
            match crate::infrastructure::tools::harness_lifecycle::current(&lifecycle) {
                HarnessLifecycleState::Accepting => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                HarnessLifecycleState::Frozen | HarnessLifecycleState::Terminated => {
                    extensions.shutdown().await;
                    return;
                }
            }
        }
    });
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
    lifetime: crate::domain::agents::harness_lifetime::HarnessLifetime,
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
