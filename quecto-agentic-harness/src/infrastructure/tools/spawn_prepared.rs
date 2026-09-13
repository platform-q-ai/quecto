//! The prepared child of one launch (#1369/#1935): what a launch adapter
//! holds between spawning (or script-managed creation) and registration —
//! the opaque owned-process handle, the environment it created, and the
//! cleanup plan a rollback consumes exactly once.
use std::sync::Arc;

use crate::application::subagents::ports::{ProtocolAttempt, TerminationConclusion};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::domain::subagent_launch::ParentEndpoint;
use crate::infrastructure::processes::child_stderr_tail::StderrTail;
#[cfg(test)]
use crate::infrastructure::processes::owned_child_supervisor::ProcessGroup;
use crate::infrastructure::processes::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor,
};
use crate::infrastructure::processes::owned_child_termination::{
    ROLLBACK_BUDGET, conclude_retained,
};

use super::cleanup_command;

/// A rolled-back child was never handed work: a short protocol budget
/// suffices before the supervisor's fallback.
const ROLLBACK_PROTOCOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug)]
pub(in crate::infrastructure::tools) struct PreparedChild {
    pub(in crate::infrastructure::tools) swarm_reservation:
        Option<super::super::swarm_admission::LaunchReservation>,
    /// Opaque handle of the locally spawned process, owned by the one
    /// supervisor from the moment it was spawned (#1935). `None` for
    /// script-managed launches: they hold no process.
    pub owned_child: Option<ChildHandleId>,
    /// Display-only pid captured at adoption (retained until #1940).
    pub display_pid: u32,
    pub supervisor: Arc<OwnedChildSupervisor>,
    pub environment_ref: Option<String>,
    /// Typed parent endpoint from the create/exec result (#1369 slice 3).
    /// `None` for local children, whose requested socket path is authoritative.
    pub endpoint: Option<ParentEndpoint>,
    /// Proxy bridge materialized at readiness; carried so registration can
    /// take ownership and rollback can abort it.
    pub proxy_bridge: Option<super::super::spawn_proxy_bridge::ProxyBridge>,
    pub process_owner: super::super::process_tree::ProcessOwner,
    pub(in crate::infrastructure::tools) cleanup_environment_id: Option<String>,
    pub(in crate::infrastructure::tools) cleanup_argv: Vec<String>,
    /// Session registry the environment was committed to, so rollback can
    /// uncommit the entry it created.
    pub(in crate::infrastructure::tools) environments: Option<EnvironmentRegistry>,
    /// Bounded tail of a locally spawned child's stderr, for the launch
    /// failure report (#1937 review). `None` for script-managed launches.
    pub stderr_tail: Option<StderrTail>,
}

/// How long a launch failure report waits for the child's stderr to reach
/// EOF after its exit was observed; a descendant holding the pipe open
/// cannot stall the report beyond this.
const STDERR_REPORT_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

impl PreparedChild {
    #[cfg(test)]
    pub(in crate::infrastructure::tools) async fn new_for_test(
        command: Option<tokio::process::Command>,
        environment_ref: Option<String>,
        endpoint: Option<ParentEndpoint>,
    ) -> Self {
        let supervisor = Arc::new(OwnedChildSupervisor::new());
        let (owned_child, display_pid, stderr_tail) = match command {
            Some(mut command) => {
                command.stderr(std::process::Stdio::piped());
                let spawned = supervisor
                    .spawn(command, ProcessGroup::Inherited)
                    .await
                    .expect("test child spawns");
                let tail = spawned
                    .stderr
                    .map(|stderr| supervisor.retain_stderr_tail(stderr));
                (Some(spawned.handle), spawned.display_pid.0, tail)
            }
            None => (None, 0, None),
        };
        Self {
            swarm_reservation: None,
            owned_child,
            display_pid,
            supervisor,
            environment_ref,
            endpoint,
            proxy_bridge: None,
            process_owner: super::super::process_tree::ProcessOwner::DirectPid,
            cleanup_environment_id: None,
            cleanup_argv: vec![],
            environments: None,
            stderr_tail,
        }
    }

    /// True when this launch created its environment (rather than joining an
    /// existing one) and therefore owns the record on rollback.
    pub fn owns_environment(&self) -> bool {
        self.environments.is_some()
    }

    pub fn cleanup_plan(&self) -> (Option<String>, Vec<String>) {
        (
            self.cleanup_environment_id.clone(),
            self.cleanup_argv.clone(),
        )
    }

    /// The child's last stderr output for a launch failure report; empty
    /// when nothing was captured. Waits (bounded) for the pipe's EOF so the
    /// refusal a child printed just before exiting is included.
    pub async fn stderr_tail_report(&self) -> String {
        let Some(tail) = &self.stderr_tail else {
            return String::new();
        };
        tail.wait_eof(STDERR_REPORT_GRACE).await;
        tail.snapshot()
    }

    /// Wait for the launched process to exit; `None` when this launch holds
    /// no process (script-managed) or the handle is unknown.
    pub async fn wait_owned_child_exit(
        &self,
    ) -> Option<crate::infrastructure::processes::owned_child_supervisor::ChildExit> {
        let handle = self.owned_child?;
        self.supervisor.wait_exit(handle).await
    }

    /// Roll back with no endpoint to ask: the protocol attempt is negative
    /// by construction.
    #[cfg(test)]
    pub async fn rollback_once(&mut self) {
        self.rollback_once_via(None).await;
    }

    /// Roll back a launch that never registered (#1936: a registered launch
    /// is compensated through `CompensateFailedLaunch`; this transaction
    /// still holds the handle of one that failed before that). When the
    /// child's requested endpoint is known the `shutdown` protocol is
    /// attempted there first (a child that became ready but whose
    /// registration failed still answers it); the same owned-handle
    /// conclusion as every other termination then applies: a signal only
    /// after a negative outcome or an exit timeout.
    pub async fn rollback_once_via(&mut self, endpoint: Option<&std::path::Path>) {
        if let Some(handle) = self.owned_child.take() {
            let attempt = match endpoint {
                Some(endpoint) => {
                    match crate::infrastructure::processes::direct_child_routing::shutdown_over_socket(
                        endpoint,
                        crate::domain::subagent_teardown::ShutdownReason::OperatorRequest,
                        ROLLBACK_PROTOCOL_TIMEOUT,
                    )
                    .await
                    {
                        Ok(()) => ProtocolAttempt::Acknowledged,
                        Err(error) => ProtocolAttempt::Negative(error.to_string()),
                    }
                }
                None => ProtocolAttempt::Negative("launch rolled back before readiness".into()),
            };
            let conclusion =
                conclude_retained(&self.supervisor, handle, attempt, ROLLBACK_BUDGET).await;
            tracing::info!(?conclusion, "rolled back launched child");
            // No reaper task ever runs for a launch that never registered:
            // the slot is retired here (or as soon as the reap completes).
            self.supervisor.retire_when_reaped(handle);
            if !matches!(
                conclusion,
                TerminationConclusion::StillRunning(_) | TerminationConclusion::NoRetainedHandle
            ) {
                if let Some(reservation) = &mut self.swarm_reservation {
                    if let Err(error) = reservation.rolled_back() {
                        tracing::error!(%error, "swarm launch rollback requires reconciliation");
                    }
                }
            }
        }
        if let Some(bridge) = self.proxy_bridge.take() {
            bridge.teardown();
        }
        run_cleanup_once(self.cleanup_environment_id.clone(), &mut self.cleanup_argv).await;
        if let (Some(environments), Some(env_ref)) = (&self.environments, &self.environment_ref) {
            environments.remove(env_ref);
        }
    }
}

pub(in crate::infrastructure::tools) async fn run_cleanup_once(
    env_ref: Option<String>,
    cleanup_argv: &mut Vec<String>,
) {
    if let Some(mut cmd) = cleanup_command(env_ref.as_deref(), cleanup_argv) {
        let _ = cmd.status().await;
        cleanup_argv.clear();
    }
}
