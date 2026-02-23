//! WASM tool isolation runtime.
//!
//! Tools run as WebAssembly components (wasm32-wasip2) executed by Wasmtime
//! with the Component Model. Each invocation gets a fresh Store — no
//! cross-call state. Tools interact with the host through the WIT interface
//! defined in `wit/tool.wit`.

pub mod capabilities;
pub mod host;
pub mod loader;
pub mod runtime;
pub mod wrapper;
