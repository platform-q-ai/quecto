//! Capability-local effect ports for subagent teardown, one module per
//! concern (#2367); importers name each port at its owning file.
//!
//! Each port is owned by a use case in this capability; infrastructure
//! implements them and composition wires concrete instances. Signatures name
//! only domain and application types: no socket, JSON, process or file
//! vocabulary crosses this boundary.

use std::future::Future;
use std::pin::Pin;

pub mod child_routing;
pub mod container_configs;
pub mod delegated_agent_registry;
pub mod ended_child_records;
pub mod harness_shutdown;
pub mod kept_environments;
pub mod owned_child_termination;
pub mod teardown_compensation;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
