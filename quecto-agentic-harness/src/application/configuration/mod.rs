//! Configuration capability (#1966, #2024): which files a run loads, how
//! the repo-local overlay merges over the global file, and the one safe
//! write path every configuration change goes through.
//!
//! The application owns the policy — selection precedence, the overlay
//! sections and the global-only ones, the merge, the single-path patch —
//! and expresses what it needs of the outside world as capability-local
//! ports: a document store, the validator, and the overlay trust decision.
//! Only composition constructs the use cases; the interface holds injected
//! handles and presents the outcomes.

pub mod dto;
pub mod overlay_policy;
pub mod ports;
pub mod redaction;
pub mod use_cases;
