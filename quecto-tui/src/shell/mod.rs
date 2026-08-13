//! TUI shell: composition root, top-level routing, and runtime/terminal adapters.
//!
//! Owns the CLI entrypoint, `App` composition/event loop, key decoding, stdin
//! buffering, and the concrete runtime adapters (terminal, render, signals,
//! process, child watch) that touch the OS.

pub mod app;
pub(crate) mod atomic_file;
pub mod child_watch;
pub mod cli;
pub(crate) mod connection;
pub mod keys;
pub mod process;
pub mod render;
pub mod signals;
pub(crate) mod socket_path;
pub mod stdin_buffer;
pub(crate) mod tab_registry;
pub mod terminal;
/// Shared test-only `tracing` warn-capture apparatus (#1112 review): used by
/// the client defence unit tests and the workspace `bdd` target.
#[cfg(any(test, feature = "test-harness"))]
pub mod warn_capture;
pub(crate) mod workspace_manifest;
