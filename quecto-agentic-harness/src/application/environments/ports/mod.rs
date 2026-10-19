//! Capability-local effect ports of the environments capability (#1939).
//!
//! Each port is owned by a use case here; infrastructure implements them and
//! composition wires concrete instances. Signatures name only domain and
//! application types: no socket, process, runtime or wire vocabulary crosses
//! this boundary. The environment registry itself is the pure domain
//! aggregate (`EnvironmentRegistry`): its exclusive kill and inspect claims
//! are the transitions these use cases drive.

use std::future::Future;
use std::pin::Pin;

pub mod container;
pub mod hosted_swarm;
pub mod process;
pub mod registry;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
