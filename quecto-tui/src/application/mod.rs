//! TUI application layer.
//!
//! This layer is reserved for orchestration and use cases over domain ports.
//! It must not perform runtime I/O and must not depend on infrastructure or
//! interface code.

pub mod model_payloads;
pub mod session_payloads;
