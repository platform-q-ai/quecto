//! Conversation ownership for `quecto-tui`.
//!
//! Owns master-history pagination, transcript recovery, rewind state, and the
//! conversation controllers. `shell::app` composes those controllers as App
//! extensions without taking ownership of conversation policy.

pub(crate) mod history_paging;
pub(crate) mod turn_recovery;
