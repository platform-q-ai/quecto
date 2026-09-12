//! Controller for the two teardown commands (#1934).
//!
//! Sequence for `shutdown`, and the only place it is decided:
//!
//! 1. claim the line (only the two teardown `type`s; everything else is
//!    ignored without a frame so the ordinary dispatcher answers it), then
//!    parse (cap, shape) and authorize the connection;
//! 2. map to the application request;
//! 3. `Prepare` — admit and freeze, receive an opaque holder token;
//! 4. write **and flush** the correlated ACK through the presenter;
//! 5. only on success hand the token to `Execute`; on failure release this
//!    holder and never execute for this command.
//!
//! Two guarantees hold across task cancellation. Before the ACK is flushed
//! the held token is guarded: dropping this future releases the holder, so
//! an aborted connection task cannot leave the harness frozen. After the ACK
//! is flushed the shutdown must complete: `Execute` detaches the run onto the
//! transaction's spawner, and this future only joins it, re-driving on an
//! interrupted run until a terminal outcome arrives.
//!
//! The controller runs on the connection's reader task, so a busy dispatch
//! loop never delays it: idle and busy harnesses take the identical path and
//! `Execute` cancels whatever turn is in flight. Authorization is a closed
//! allowlist over what the connection layer established: `BoundParent`
//! arrives with the launch-bound parent connection (#1935); `LocalOperator`
//! is today satisfiable by any client of the harness's own socket because no
//! peer authentication exists on it yet. No teardown policy lives here — the
//! controller neither decides what shutdown does nor whether a target may be
//! reached; it sequences the use cases and presents results.
use std::sync::Arc;

use crate::application::subagents::dto::{
    HarnessShutdownError, PrepareShutdownRequest, ReleaseOutcome, ShutdownOutcome,
    TerminateDelegatedAgentError, TerminationRouted,
};
use crate::application::subagents::use_cases::{
    ExecuteHarnessShutdown, PrepareHarnessShutdown, TerminateDelegatedAgent,
};

use super::mapping::{TeardownRequest, map_command};
use super::presenter::{
    AckWriteError, AckWriter, deliver, edge_rejection, shutdown_ack, shutdown_rejection,
    termination_rejection, termination_response,
};
use super::wire::{
    SHUTDOWN_COMMAND, TERMINATE_DELEGATED_AGENT_COMMAND, WireError, parse_teardown_command,
};

/// Who is on the other end of the connection, as established by the
/// connection layer. Only the allowlisted authorities may tear down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionAuthority {
    /// The authenticated launch-bound parent of this harness.
    BoundParent,
    /// A local operator client (TUI/CLI) on the harness's own socket.
    LocalOperator,
    /// Anything that has not proven either of the above.
    Unauthenticated,
}

impl ConnectionAuthority {
    pub const fn may_tear_down(self) -> bool {
        matches!(self, Self::BoundParent | Self::LocalOperator)
    }
}

/// Whether the dispatch loop was busy when the command arrived. Recorded
/// for the outcome; it never changes the path taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Idle,
    Busy,
}

/// Command name used for rejections raised before the command was parsed.
pub const UNPARSED_COMMAND: &str = "teardown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerOutcome {
    /// Not a teardown command; the caller dispatches it as usual.
    Ignored,
    /// Refused before any use case ran. `written` reports the rejection frame.
    Rejected {
        command: &'static str,
        detail: String,
        written: Result<(), AckWriteError>,
    },
    /// ACK flushed, then executed (or joined) for the admitted token.
    ShutdownExecuted {
        delivery: DeliveryState,
        outcome: Result<ShutdownOutcome, HarnessShutdownError>,
    },
    /// ACK could not be delivered: the admission was released instead and
    /// `Execute` was never called for this command.
    ShutdownAbandoned {
        ack_error: AckWriteError,
        release: Result<ReleaseOutcome, HarnessShutdownError>,
    },
    TerminationRouted {
        delivery: DeliveryState,
        outcome: Result<TerminationRouted, TerminateDelegatedAgentError>,
        written: Result<(), AckWriteError>,
    },
}

pub struct SubagentTeardownController {
    prepare: Arc<PrepareHarnessShutdown>,
    execute: Arc<ExecuteHarnessShutdown>,
    terminate: Arc<TerminateDelegatedAgent>,
}

impl SubagentTeardownController {
    pub fn new(
        prepare: Arc<PrepareHarnessShutdown>,
        execute: Arc<ExecuteHarnessShutdown>,
        terminate: Arc<TerminateDelegatedAgent>,
    ) -> Self {
        Self {
            prepare,
            execute,
            terminate,
        }
    }

    pub async fn handle(
        &self,
        line: &str,
        authority: ConnectionAuthority,
        delivery: DeliveryState,
        writer: &dyn AckWriter,
    ) -> ControllerOutcome {
        let command = match parse_teardown_command(line) {
            Ok(command) => command,
            Err(WireError::NotATeardownCommand) => return ControllerOutcome::Ignored,
            Err(error) => {
                let id = match &error {
                    WireError::Malformed { id, .. } | WireError::Oversized { id, .. } => id.clone(),
                    WireError::NotATeardownCommand => None,
                };
                return reject(writer, id.as_deref(), UNPARSED_COMMAND, error.to_string()).await;
            }
        };
        let id = command.id().map(str::to_owned);
        let name = command.command_name();
        if !authority.may_tear_down() {
            return reject(
                writer,
                id.as_deref(),
                name,
                "unauthorized connection".to_owned(),
            )
            .await;
        }
        let request = match map_command(&command) {
            Ok(request) => request,
            Err(error) => return reject(writer, id.as_deref(), name, error.to_string()).await,
        };
        match request {
            TeardownRequest::Shutdown(request) => {
                self.shutdown(id.as_deref(), request, delivery, writer)
                    .await
            }
            TeardownRequest::TerminateDelegatedAgent(request) => {
                let outcome = self.terminate.execute(request).await;
                let response = match &outcome {
                    Ok(routed) => termination_response(id.as_deref(), routed),
                    Err(error) => termination_rejection(id.as_deref(), error),
                };
                let written = deliver(writer, &response).await;
                ControllerOutcome::TerminationRouted {
                    delivery,
                    outcome,
                    written,
                }
            }
        }
    }

    async fn shutdown(
        &self,
        id: Option<&str>,
        request: PrepareShutdownRequest,
        delivery: DeliveryState,
        writer: &dyn AckWriter,
    ) -> ControllerOutcome {
        let prepared = match self.prepare.execute(request) {
            Ok(prepared) => prepared,
            Err(error) => {
                let written = deliver(writer, &shutdown_rejection(id, &error)).await;
                return ControllerOutcome::Rejected {
                    command: SHUTDOWN_COMMAND,
                    detail: error.to_string(),
                    written,
                };
            }
        };
        // Until the ACK is flushed this holder is guarded: if the connection
        // task is dropped mid-write, the guard releases it and the harness is
        // not left frozen.
        let mut held = HeldAdmission {
            prepare: &self.prepare,
            token: Some(prepared.token.clone()),
        };
        // The ACK must be on the wire before the token can move: a parent
        // that later sees EOF then knows it was a graceful exit.
        if let Err(ack_error) = deliver(writer, &shutdown_ack(id, &prepared)).await {
            let release = held.release();
            return ControllerOutcome::ShutdownAbandoned { ack_error, release };
        }
        // From here the shutdown must complete: the run is detached inside
        // `Execute` (spawner port) and this future only joins it.
        held.disarm();
        let outcome = self.execute_to_completion(&prepared.token).await;
        ControllerOutcome::ShutdownExecuted { delivery, outcome }
    }

    /// Join the detached run, re-driving it if its runtime dropped it. The
    /// bound keeps a spawner that drops every run from spinning forever.
    async fn execute_to_completion(
        &self,
        token: &crate::application::subagents::dto::ShutdownToken,
    ) -> Result<ShutdownOutcome, HarnessShutdownError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.execute.execute(token).await {
                Err(HarnessShutdownError::ExecutionInterrupted) if attempt < MAX_REDRIVES => {
                    continue;
                }
                outcome => return outcome,
            }
        }
    }
}

/// Upper bound on re-driving an interrupted run for one command.
pub const MAX_REDRIVES: u32 = 4;

/// One holder's token between `Prepare` and the flushed ACK. Dropped before
/// `disarm`, it releases the holder; the freeze lifts if it was the last.
struct HeldAdmission<'a> {
    prepare: &'a PrepareHarnessShutdown,
    token: Option<crate::application::subagents::dto::ShutdownToken>,
}

impl HeldAdmission<'_> {
    fn release(&mut self) -> Result<ReleaseOutcome, HarnessShutdownError> {
        let token = self.token.take().expect("released at most once");
        self.prepare.release(&token)
    }

    fn disarm(&mut self) {
        self.token = None;
    }
}

impl Drop for HeldAdmission<'_> {
    fn drop(&mut self) {
        if self.token.is_some() {
            // Best effort: the outcome has no one left to report to.
            let _ = self.release();
        }
    }
}

async fn reject(
    writer: &dyn AckWriter,
    id: Option<&str>,
    command: &'static str,
    detail: String,
) -> ControllerOutcome {
    debug_assert!(
        matches!(
            command,
            SHUTDOWN_COMMAND | TERMINATE_DELEGATED_AGENT_COMMAND | UNPARSED_COMMAND
        ),
        "rejections name a known command"
    );
    let written = deliver(writer, &edge_rejection(id, command, detail.clone())).await;
    ControllerOutcome::Rejected {
        command,
        detail,
        written,
    }
}

#[cfg(test)]
#[path = "controller_rig_tests.rs"]
mod rig;
#[cfg(test)]
#[path = "controller_routing_tests.rs"]
mod routing_tests;
#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
