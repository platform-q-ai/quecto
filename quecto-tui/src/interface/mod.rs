//! TUI interface layer.
//!
//! This layer is the composition root for executable entry points and user-facing
//! wiring. It may depend on all inner layers.

pub mod ansi;
pub mod app;
pub mod cli;
pub mod component;
pub mod components;
pub(crate) mod feed_state;
pub mod fuzzy;
pub mod keys;
pub mod kitty;
pub(crate) mod ledger_sync;
pub mod overlay;
pub mod select_overlay;
pub mod stdin_buffer;
pub mod theme;
pub mod utils;
