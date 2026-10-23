//! Unix-domain-socket delivery adapter: wire DTOs, mapping, controllers and
//! presenters. Parses, maps, invokes one use case, presents. No policy.

pub mod admission;
pub mod catalogue;
pub mod controllers;
pub mod framing;
pub mod parent_control;
pub mod sessions;
pub mod subagent_teardown;
pub mod wire;
