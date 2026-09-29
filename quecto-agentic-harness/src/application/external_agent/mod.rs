//! An external agent run as a member's "brain" (epic #2284). This slice
//! (#2285) holds the capability's DTOs and the projection of its event
//! stream onto them: a capability-internal helper, not a use case (the
//! session use case of a later slice builds one per session). #2286 adds
//! the process ports its adapter implements; #2287 the session use case
//! that drives them.

pub mod dto;
pub mod event_log;
pub mod ports;
pub mod projection;
mod session_core;
mod session_telemetry;
pub mod use_cases;
