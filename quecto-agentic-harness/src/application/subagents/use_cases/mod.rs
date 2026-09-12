//! Subagent teardown use cases.

pub mod compensate_failed_launch;
pub mod harness_shutdown;
pub mod kill_delegated_agent;
pub mod observe_owned_child_exit;
pub mod terminate_delegated_agent;

pub use compensate_failed_launch::{CompensateFailedLaunch, CompensateFailedLaunchPorts};
pub use harness_shutdown::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown,
};
pub use kill_delegated_agent::{KillDelegatedAgent, KillDelegatedAgentPorts};
pub use observe_owned_child_exit::ObserveOwnedChildExit;
pub use terminate_delegated_agent::TerminateDelegatedAgent;

#[cfg(test)]
#[path = "lifecycle_fakes_tests.rs"]
pub(crate) mod lifecycle_fakes;
#[cfg(test)]
#[path = "teardown_fakes_tests.rs"]
pub(crate) mod teardown_fakes;
