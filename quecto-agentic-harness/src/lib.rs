#![deny(dead_code)]
#![deny(unused_imports)]

pub mod application;
pub use application::environment_control as environment_control_app;
pub use application::subagent_launch as subagent_launch_app;
pub mod domain;
pub mod infrastructure;
pub mod interface;

// Execute the same public-port contracts in the library-only coverage gate as
// well as the integration contract binary; keep one behavioral source of truth.
#[cfg(test)]
extern crate self as quecto;
#[cfg(test)]
#[path = "../tests/contracts/admission_client.rs"]
mod admission_client_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_dispatcher.rs"]
mod admission_dispatcher_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_registry.rs"]
mod admission_registry_contracts;
