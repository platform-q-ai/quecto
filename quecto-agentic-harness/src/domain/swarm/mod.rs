//! Swarm lifecycle vocabulary, independent of adapter protocols. The effect
//! ports (`CoordinationPort`, `SwarmRunControl`, `ProcessControl`, …) are the
//! application's (`application::swarm::ports`, #1940, #1960).
//!
//! The coordination board's pure decisions (#2265, #2266) live in the
//! submodules: value records, the policy ported from `swarm_policy.py`, and
//! input validation, wake notification, owner liveness and the usage
//! budget. No I/O and no storage format.

pub mod entities;
pub mod events;
pub mod services;
pub mod value_objects;
