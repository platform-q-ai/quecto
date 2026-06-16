//! TUI infrastructure layer.
//!
//! This layer is reserved for concrete adapters such as terminal, process,
//! signal, and UDS implementations. It may depend inward on domain and
//! application code, but not on the interface composition root.
