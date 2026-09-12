//! Per-connection launch-bound parent control and teardown edge (#1935).
//!
//! Every client connection of the multi-client loop runs its incoming lines
//! through [`intercept_line`] before ordinary dispatch:
//!
//! 1. a `bind_parent_control` presentation is compared against the binding
//!    this harness was launched with (affirmative allowlist: exactly one
//!    connection may bind; missing, mismatched, replayed or second
//!    presentations **close the connection**, never demote it to an
//!    ordinary client);
//! 2. the two teardown commands go to the slice A controller with the
//!    authority this connection proved (`BoundParent` for the bound one,
//!    `LocalOperator` for any other client of the harness's own socket);
//! 3. everything else is not claimed and dispatches as before.
//!
//! When a connection's reader ends, [`connection_closed`] asks the binding
//! whether that connection was the bound parent. Only then does the common
//! shutdown run, with reason `parent_connection_lost`, through the same
//! controller — and only once, because the binding moves to `Lost`.
//! Ordinary TUI/inspector/tool/probe disconnects (including the last one)
//! never get here past the binding's refusal.
use std::sync::{Arc, Mutex};

use crate::domain::parent_control::{ConnectionLoss, ParentControlBinding};
use crate::interface::uds::parent_control::wire::{BindParentControlWire, bound_ack_line};
use crate::interface::uds::subagent_teardown::controller::{
    ConnectionAuthority, ControllerOutcome, DeliveryState, SubagentTeardownController,
};
use crate::interface::uds::subagent_teardown::wire::{
    SHUTDOWN_COMMAND, TERMINATE_DELEGATED_AGENT_COMMAND,
};

use super::uds_multi::BusyFlag;
use super::uds_teardown_adapters::{ConnectionAckWriter, SharedWriter};
use super::uds_wire::ConnectionWireMode;

/// Shared across every connection of one harness.
pub struct ConnectionTeardown {
    pub binding: Arc<Mutex<ParentControlBinding>>,
    pub controller: Arc<SubagentTeardownController>,
    pub busy: BusyFlag,
}

impl ConnectionTeardown {
    fn delivery(&self) -> DeliveryState {
        if self.busy.load(std::sync::atomic::Ordering::SeqCst) {
            DeliveryState::Busy
        } else {
            DeliveryState::Idle
        }
    }

    pub fn binding_state(&self) -> crate::domain::parent_control::BindingState {
        self.binding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .state()
    }
}

/// What one connection has proven so far.
#[derive(Debug, Default)]
pub struct ConnectionRole {
    bound_parent: bool,
}

impl ConnectionRole {
    pub fn authority(&self) -> ConnectionAuthority {
        if self.bound_parent {
            ConnectionAuthority::BoundParent
        } else {
            ConnectionAuthority::LocalOperator
        }
    }

    pub fn is_bound_parent(&self) -> bool {
        self.bound_parent
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Intercept {
    /// Not a presentation or teardown command: dispatch as usual.
    NotClaimed,
    /// Fully handled on this connection.
    Handled,
    /// Fail closed: the connection must be closed now.
    Close,
}

pub(crate) struct LineContext<'a> {
    pub(crate) teardown: &'a ConnectionTeardown,
    pub(crate) role: &'a mut ConnectionRole,
    pub(crate) writer: &'a SharedWriter,
    pub(crate) wire_mode: &'a ConnectionWireMode,
    pub(crate) client_id: u64,
}

/// Cheap prefilter so an ordinary line — a prompt, a token stream, a frame of
/// up to `MAX_FRAME_PAYLOAD_BYTES` — is never parsed here at all: only a
/// line that could be a presentation or one of the two teardown commands is
/// handed to the claim parsers.
///
/// A `type` written with JSON `\u00xx` escapes would slip past a plain
/// substring check, so any line carrying such an escape is parsed in full:
/// the fail-closed property holds whatever the spelling.
pub(crate) fn may_be_control_line(line: &str) -> bool {
    BindParentControlWire::may_be_presentation(line)
        || line.contains(SHUTDOWN_COMMAND)
        || line.contains(TERMINATE_DELEGATED_AGENT_COMMAND)
        || line.contains("\\u00")
}

pub(crate) async fn intercept_line(mut ctx: LineContext<'_>, line: &str) -> Intercept {
    if !may_be_control_line(line) {
        return Intercept::NotClaimed;
    }
    match BindParentControlWire::claim(line) {
        Ok(None) => {}
        Ok(Some(presentation)) => return bind(&mut ctx, &presentation).await,
        Err(detail) => {
            tracing::warn!(client_id = ctx.client_id, %detail, "closing connection: malformed parent control presentation");
            return Intercept::Close;
        }
    }
    let writer = ConnectionAckWriter {
        writer: Arc::clone(ctx.writer),
        mode: ctx.wire_mode.clone(),
    };
    let outcome = ctx
        .teardown
        .controller
        .handle(line, ctx.role.authority(), ctx.teardown.delivery(), &writer)
        .await;
    match outcome {
        ControllerOutcome::Ignored => Intercept::NotClaimed,
        other => {
            tracing::info!(client_id = ctx.client_id, outcome = ?redact(&other), "teardown command handled");
            Intercept::Handled
        }
    }
}

/// The controller outcome without any field that could carry a line.
fn redact(outcome: &ControllerOutcome) -> &'static str {
    match outcome {
        ControllerOutcome::Ignored => "ignored",
        ControllerOutcome::Rejected { .. } => "rejected",
        ControllerOutcome::ShutdownExecuted { .. } => "shutdown executed",
        ControllerOutcome::ShutdownAbandoned { .. } => "shutdown abandoned",
        ControllerOutcome::TerminationRouted { .. } => "termination routed",
    }
}

async fn bind(ctx: &mut LineContext<'_>, presentation: &BindParentControlWire) -> Intercept {
    let presented = match presentation.presented() {
        Ok(presented) => presented,
        Err(detail) => {
            tracing::warn!(client_id = ctx.client_id, %detail, "closing connection: invalid parent control presentation");
            return Intercept::Close;
        }
    };
    let accepted = {
        let mut binding = ctx
            .teardown
            .binding
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        binding.present(presented.0, &presented.1)
    };
    match accepted {
        Ok(()) => {
            ctx.role.bound_parent = true;
            // The ACK is informational for the listening parent; a failed
            // write surfaces as EOF on its side like any other write error.
            let mut writer = ctx.writer.lock().await;
            if let Err(error) =
                super::uds_wire::write_event_line(&mut *writer, &bound_ack_line(), ctx.wire_mode)
                    .await
            {
                tracing::debug!(client_id = ctx.client_id, %error, "parent control ack not delivered");
            }
            tracing::info!(
                client_id = ctx.client_id,
                "launch-bound parent control connection bound"
            );
            Intercept::Handled
        }
        Err(rejection) => {
            tracing::warn!(client_id = ctx.client_id, %rejection, "closing connection: parent control presentation refused");
            Intercept::Close
        }
    }
}

/// The connection's reader ended. Returns the controller outcome when this
/// was the bound parent (the common shutdown ran), `None` otherwise.
pub(crate) async fn connection_closed(
    teardown: &ConnectionTeardown,
    role: &ConnectionRole,
    client_id: u64,
) -> Option<ControllerOutcome> {
    let loss = teardown
        .binding
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .connection_closed(role.bound_parent);
    match loss {
        ConnectionLoss::OrdinaryClient => None,
        ConnectionLoss::BoundParentLost => {
            tracing::warn!(
                client_id,
                "launch-bound parent connection lost; running common shutdown"
            );
            Some(
                teardown
                    .controller
                    .parent_connection_lost(teardown.delivery())
                    .await,
            )
        }
    }
}

/// Arm the bind deadline of a launched harness: if no parent has bound by
/// then, the binding is spent and the common shutdown runs with reason
/// `parent_never_bound`. A harness that was bound (or already lost its
/// parent) in time is untouched. Returns the watcher's handle.
pub(crate) fn arm_bind_deadline(
    teardown: Arc<ConnectionTeardown>,
    deadline: super::uds_teardown_graph::BindDeadline,
) -> tokio::task::JoinHandle<Option<ControllerOutcome>> {
    tokio::spawn(async move {
        match deadline {
            super::uds_teardown_graph::BindDeadline::After(duration) => {
                tokio::time::sleep(duration).await;
            }
            super::uds_teardown_graph::BindDeadline::Triggered(trigger) => {
                trigger.notified().await;
            }
        }
        bind_deadline_passed(&teardown).await
    })
}

/// The deadline elapsed: expire an `Unbound` launched binding and run the
/// common shutdown exactly once; anything else is a no-op.
pub(crate) async fn bind_deadline_passed(
    teardown: &ConnectionTeardown,
) -> Option<ControllerOutcome> {
    let expired = teardown
        .binding
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .expire_unbound();
    if !expired {
        return None;
    }
    tracing::warn!(
        "no parent bound its control connection before the deadline; running common shutdown"
    );
    Some(
        teardown
            .controller
            .parent_never_bound(teardown.delivery())
            .await,
    )
}

#[cfg(test)]
#[path = "uds_parent_control_tests.rs"]
mod tests;
