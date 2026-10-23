//! Unix-domain-socket delivery adapter: wire DTOs, mapping, controllers and
//! presenters. Parses, maps, invokes one use case, presents. No policy.

pub mod admission;
pub mod controllers;
pub mod framing;
pub mod mapping;
pub mod presenters;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod wire;
