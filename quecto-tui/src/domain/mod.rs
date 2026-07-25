//! TUI domain layer.
//!
//! This layer is reserved for pure TUI vocabulary, value objects, and ports.
//! It must not perform runtime I/O and must not depend on application,
//! infrastructure, or interface code.

pub mod history_paging;
pub mod turn_recovery;
