//! Sessions capability (#1960, #1968): the persistence ports the session
//! lifecycle and context recall depend on, the boundary DTOs, the
//! application-owned state of the loop's active session, and the use cases
//! that own the saved-session transactions and the live-conversation reads.
//! The pure session vocabulary is the domain's (`domain::session`,
//! `domain::session_identity`, `domain::conversation_view`).
//!
//! Only composition (`composition::sessions`) constructs the use cases and
//! the active-session state; interface and infrastructure hold injected
//! handles.

pub mod active_session;
pub mod catalogue;
pub mod conversation_ledger;
pub mod dto;
pub mod history_paging;
pub mod ports;
pub mod production_resume;
pub mod resume_decision;
pub mod use_cases;
