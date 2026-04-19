//! Contract tests for the `AgentLoop` port.
//!
//! Any adapter implementing `AgentLoop` must satisfy these assertions.
//! Adapter-specific behaviour belongs in unit tests close to each adapter;
//! this file exists so chainlink can verify the boundary carries a committed
//! contract. Extend with scenario tests that drive each adapter through the
//! port API as the contract surface expands.

use quecto::domain::agent::AgentLoop;

/// Object-safety is part of the port contract: the composition root stores
/// `Arc<dyn AgentLoop>` and dynamically dispatches. If the trait loses object
/// safety, this file stops compiling — failing the contract at build time.
#[test]
fn port_is_object_safe() {
    fn _accepts_trait_object(_: &dyn AgentLoop) {}
}
