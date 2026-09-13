//! Termination-signal and last-client delivery for the UDS harness (#1938).
//!
//! This module only delivers triggers. SIGTERM/SIGINT and the last client's
//! disconnect (for the top-level lifetime that ends with it) are handed to
//! the one [`SubagentTeardownController`], whose common shutdown — admit and
//! freeze, cancel the turn, tear down the fleet of direct children under a
//! bound, persist, signal exit readiness — is the same one a `shutdown`
//! command or the bound parent's loss runs. No registry is drained here and
//! no process is signalled here; the loop-exit `Notify` this module owns is
//! what the composition exit-readiness adapter fires when the teardown has
//! settled, so the dispatch loop returns only after the subtree is gone.
//!
//! A harness built without a teardown graph (unit rigs) still exits cleanly
//! on a signal: the turn is cancelled and the loop is told to finish.
use std::future::Future;
use std::sync::Arc;

use tokio::sync::Notify;

use crate::interface::uds::subagent_teardown::controller::{
    ControllerOutcome, DeliveryState, SubagentTeardownController,
};

use super::uds_cancel::{CancelHandle, fire_cancel};
use super::uds_multi::BusyFlag;

/// Handle the dispatch loop waits on; resolves once a shutdown has settled.
pub(super) struct ShutdownRequest {
    notify: Arc<Notify>,
    controller: Option<Arc<SubagentTeardownController>>,
    busy: BusyFlag,
}

impl ShutdownRequest {
    /// Install the signal watcher. Delivery happens on the watcher task,
    /// never on the dispatch loop, so a busy turn cannot delay it.
    pub(super) fn install(
        notify: Arc<Notify>,
        controller: Option<Arc<SubagentTeardownController>>,
        busy: BusyFlag,
        cancel_handle: CancelHandle,
    ) -> Self {
        tokio::spawn(shutdown_on(
            termination_signal(),
            controller.clone(),
            busy.clone(),
            cancel_handle,
            notify.clone(),
        ));
        Self {
            notify,
            controller,
            busy,
        }
    }

    /// Resolves after a shutdown has settled. A request that arrives before
    /// anyone waits is retained (`Notify` keeps one permit).
    pub(super) async fn requested(&self) {
        self.notify.notified().await;
    }

    /// The last client of a harness whose lifetime ends with it left: run
    /// the common shutdown to completion before the loop returns. Without a
    /// controller there is nothing to tear down and the loop may return.
    pub(super) async fn last_client_disconnected(&self) -> Option<ControllerOutcome> {
        let controller = self.controller.as_ref()?;
        let outcome = controller
            .last_client_disconnected(delivery(&self.busy))
            .await;
        tracing::info!(outcome = ?summary(&outcome), "last client disconnected; shutdown settled");
        Some(outcome)
    }

    /// A request with no controller and no watcher: unit rigs of the
    /// dispatch loop drive `notify` themselves.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn detached(notify: Arc<Notify>) -> Self {
        Self {
            notify,
            controller: None,
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

fn delivery(busy: &BusyFlag) -> DeliveryState {
    if busy.load(std::sync::atomic::Ordering::SeqCst) {
        DeliveryState::Busy
    } else {
        DeliveryState::Idle
    }
}

fn summary(outcome: &ControllerOutcome) -> &'static str {
    match outcome {
        ControllerOutcome::Ignored => "ignored",
        ControllerOutcome::Rejected { .. } => "rejected",
        ControllerOutcome::ShutdownExecuted { .. } => "shutdown executed",
        ControllerOutcome::ShutdownAbandoned { .. } => "shutdown abandoned",
        ControllerOutcome::TerminationRouted { .. } => "termination routed",
    }
}

/// Resolves on the first SIGTERM or SIGINT. If a handler cannot be
/// registered, never resolves — the process then keeps today's default
/// behaviour rather than failing to start. Under test support the same
/// future also resolves on [`test_support::deliver_termination_signal`], so
/// an in-process harness can be driven through the signal path without
/// signalling the test process.
async fn termination_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        tracing::warn!(
            "could not register termination signal handlers; subagents will not be torn down on exit"
        );
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
        () = test_support::simulated_signal() => {}
    }
}

/// Await `trigger`, then deliver the termination signal to the controller:
/// the common shutdown cancels the turn, tears down the fleet, persists and
/// signals exit readiness, which fires `notify`. The loop is told to finish
/// afterwards in every case, so a signal on an already-terminated harness
/// still ends the process.
pub(super) async fn shutdown_on(
    trigger: impl Future<Output = ()>,
    controller: Option<Arc<SubagentTeardownController>>,
    busy: BusyFlag,
    cancel_handle: CancelHandle,
    notify: Arc<Notify>,
) -> Option<ControllerOutcome> {
    trigger.await;
    tracing::info!("termination signal received; running the common shutdown");
    let outcome = match controller {
        Some(controller) => Some(controller.termination_signal(delivery(&busy)).await),
        None => {
            fire_cancel(&cancel_handle);
            None
        }
    };
    if let Some(outcome) = outcome.as_ref() {
        tracing::info!(outcome = summary(outcome), "termination shutdown settled");
    }
    notify.notify_one();
    outcome
}

/// Drives the signal path without signalling the test process.
pub mod test_support {
    #[cfg(any(test, feature = "test-support"))]
    static SIMULATED: tokio::sync::Notify = tokio::sync::Notify::const_new();

    /// Resolves when a simulated termination signal is delivered; never
    /// resolves in a production build.
    pub(super) async fn simulated_signal() {
        #[cfg(any(test, feature = "test-support"))]
        {
            SIMULATED.notified().await;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            std::future::pending::<()>().await;
        }
    }

    /// Deliver a simulated SIGTERM to every installed watcher of this
    /// process.
    #[cfg(any(test, feature = "test-support"))]
    pub fn deliver_termination_signal() {
        SIMULATED.notify_waiters();
    }
}

#[cfg(test)]
#[path = "uds_shutdown_tests.rs"]
mod tests;
