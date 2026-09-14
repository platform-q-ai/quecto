//! Sessions capability (#1960, #1968): the persistence ports the session
//! lifecycle and context recall depend on, the boundary DTOs, and the
//! use cases that own the saved-session transactions. The pure session
//! vocabulary is the domain's (`domain::session`, `domain::session_identity`).
//!
//! Only composition (`composition::sessions`) constructs the use cases;
//! interface and infrastructure hold injected handles.

pub mod dto;
pub mod ports;
pub mod use_cases;
