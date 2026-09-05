//! Termination-signal teardown for the UDS harness.
//!
//! The TUI's ordinary exit (Ctrl-D) persists the session and then SIGTERMs
//! the harness process group. Host-spawned children live in that group, but
//! container environments do not: their processes hang off the container
//! runtime, and the only thing that can reach them is this harness running
//! each environment's retained `kill` argv. Before this module the harness
//! died on the signal's default action, so every container environment
//! outlived all of its clients (observed: four agents in a container still
//! serving sockets minutes after the TUI had gone).
//!
//! On SIGTERM/SIGINT the watcher runs exactly the teardown
//! `delete_all_subagents` runs — host process trees and environment kills
//! — BEFORE anything else, then cancels any in-flight turn and asks the
//! dispatch loop to finish so the session is saved with an empty roster.
use std::future::Future;
use std::sync::Arc;

use tokio::sync::Notify;

use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

use super::uds_cancel::{CancelHandle, fire_cancel};

/// Handle the dispatch loop waits on; resolves once the teardown has run.
pub(super) struct ShutdownRequest {
    notify: Arc<Notify>,
}

impl ShutdownRequest {
    /// Install the watcher. Teardown happens on the watcher task, never on
    /// the dispatch loop, so a busy turn cannot delay it.
    pub(super) fn install(
        registry: Option<SubagentRegistry>,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
        cancel_handle: CancelHandle,
    ) -> Self {
        let notify = Arc::new(Notify::new());
        // A harness without a registry has nothing to tear down but must
        // still exit cleanly on the signal.
        let registry =
            registry.unwrap_or_else(|| Arc::new(std::sync::Mutex::new(Default::default())));
        tokio::spawn(shutdown_on(
            termination_signal(),
            registry,
            broadcast_tx,
            cancel_handle,
            notify.clone(),
        ));
        Self { notify }
    }

    /// Resolves after the teardown has completed. A request that arrives
    /// before anyone waits is retained (`Notify` keeps one permit).
    pub(super) async fn requested(&self) {
        self.notify.notified().await;
    }

    #[cfg(test)]
    pub(super) fn for_tests() -> (Self, Arc<Notify>) {
        let notify = Arc::new(Notify::new());
        (
            Self {
                notify: notify.clone(),
            },
            notify,
        )
    }
}

/// Resolves on the first SIGTERM or SIGINT. If a handler cannot be
/// registered, never resolves — the process then keeps today's default
/// behaviour rather than failing to start.
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
    }
}

/// Await `trigger`, then tear down every live subagent and environment,
/// cancel the in-flight turn, and notify the dispatch loop. Returns the
/// number of registry entries removed.
pub(super) async fn shutdown_on(
    trigger: impl Future<Output = ()>,
    registry: SubagentRegistry,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    cancel_handle: CancelHandle,
    notify: Arc<Notify>,
) -> usize {
    trigger.await;
    tracing::info!("termination signal received; tearing down subagents and environments");
    // The retained kill scripts run synchronously; keep them off the runtime
    // workers so the dispatch loop and client writers stay responsive.
    let removed = tokio::task::spawn_blocking(move || {
        super::uds_delete_all_subagents::delete_all_subagents_from_registry(
            &registry,
            broadcast_tx.as_ref(),
        )
    })
    .await
    .unwrap_or(0);
    tracing::info!(
        removed,
        "termination teardown complete; requesting dispatch loop exit"
    );
    fire_cancel(&cancel_handle);
    notify.notify_one();
    removed
}

#[cfg(test)]
#[path = "uds_shutdown_tests.rs"]
mod tests;
