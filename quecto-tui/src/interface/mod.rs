//! TUI interface layer.
//!
//! This layer is the composition root for executable entry points and user-facing
//! wiring. It may depend on all inner layers.

pub mod app;
pub mod cli;
pub mod client;
pub mod component;
pub mod components;
pub mod extension;
pub mod fuzzy;
pub mod keys;
pub mod kitty;
pub mod overlay;
pub mod process;
pub mod render;
pub mod signals;
pub mod stdin_buffer;
pub mod terminal;
pub mod theme;
pub mod themes;
pub mod utils;
