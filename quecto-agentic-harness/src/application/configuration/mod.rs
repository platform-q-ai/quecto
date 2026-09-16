//! Configuration capability (#1966): which configuration file a run loads.
//!
//! The application owns the selection policy — explicit override, then the
//! working directory's `config.json`, then the global file — and expresses
//! the one filesystem fact it needs (is a local file present and usable?)
//! as a capability-local port. Only composition constructs the use case;
//! the interface holds an injected handle and presents the outcome.

pub mod dto;
pub mod ports;
pub mod use_cases;
