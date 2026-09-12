//! UDS edge for the two subagent teardown operations (#1934).

pub mod controller;
pub mod mapping;
pub mod presenter;
pub mod wire;

#[cfg(test)]
#[path = "ack_fakes_tests.rs"]
pub(crate) mod ack_fakes;
