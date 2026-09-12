//! Subagent teardown use cases.

pub mod harness_shutdown;
pub mod terminate_delegated_agent;

pub use harness_shutdown::{
    ExecuteHarnessShutdown, ExecuteHarnessShutdownPorts, HarnessShutdownTransaction,
    PrepareHarnessShutdown,
};
pub use terminate_delegated_agent::TerminateDelegatedAgent;

#[cfg(test)]
#[path = "teardown_fakes_tests.rs"]
pub(crate) mod teardown_fakes;
