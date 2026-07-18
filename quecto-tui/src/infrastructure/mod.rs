//! TUI infrastructure layer.
//!
//! This layer contains concrete adapters such as terminal, process, signal, and
//! UDS implementations. It may depend inward on domain and application code, but
//! not on the interface composition root.

pub mod child_watch;
pub mod client;
pub mod process;
pub mod render;
pub mod signals;
pub mod terminal;
/// Shared test-only `tracing` warn-capture apparatus (#1112 review): used by
/// the client defence unit tests and the workspace `bdd` target.
#[cfg(any(test, feature = "test-harness"))]
pub mod warn_capture;
pub mod workspace_files;
