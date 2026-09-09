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
#[path = "../tests/contracts/admission_journal.rs"]
mod admission_journal_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_recovery.rs"]
mod admission_recovery_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_secret_source.rs"]
mod admission_secret_source_contracts;
// The authority process adapters are proven over real sockets/files; run that
// suite in the library gate too so its adapters count toward coverage.
#[cfg(test)]
#[path = "../tests/contracts/admission_fallback.rs"]
mod admission_fallback_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_feedback.rs"]
mod admission_feedback_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_group_fallback.rs"]
mod admission_group_fallback_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_registry.rs"]
mod admission_registry_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_typed_feedback_cases.rs"]
mod admission_typed_feedback_cases_contracts;
#[cfg(test)]
#[path = "../tests/contracts/admission_typed_feedback.rs"]
mod admission_typed_feedback_contracts;
#[cfg(test)]
#[path = "../tests/contracts/attempt_admission.rs"]
mod attempt_admission_contracts;
#[cfg(test)]
#[path = "../tests/contracts/attempt_permit.rs"]
mod attempt_permit_contracts;
#[cfg(all(test, unix))]
#[path = "../tests/inference_admission_broker.rs"]
mod inference_admission_broker_cov;
