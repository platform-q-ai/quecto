//! Use cases of the configuration capability. Constructed only by
//! `composition::configuration`; the interface holds an injected handle.

pub mod select_config;

pub use select_config::SelectConfig;
