//! TUI infrastructure layer.
//!
//! This layer contains concrete adapters such as terminal, process, signal, and
//! UDS implementations. It may depend inward on domain and application code, but
//! not on the interface composition root.

pub mod client;
pub mod workspace_files;
