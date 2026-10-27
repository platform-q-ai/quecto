//! Test-support entry points over the UDS adapter (#2370), used by BDD
//! scenarios and integration tests. Built only under `cfg(test)` or the
//! `test-support` feature.
mod uds_busy_test_support;
mod uds_test_support;
#[cfg(test)]
pub(crate) use uds_busy_test_support::SIMULATED_SIGNAL;
pub use uds_busy_test_support::{busy_reader_intercept, busy_reader_intercept_with_fleet};
pub use uds_test_support::{
    AdmissionStateProbe, admission_broadcast_hook, busy_reader_dispatch,
    completed_live_execution_state, ledger_hint_lines_for_turn_events,
    live_execution_state_for_events,
};
