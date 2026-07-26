//! Conversation ownership for `quecto-tui`.
//!
//! Owns master-history pagination, transcript recovery, rewind state, and the
//! conversation app slices while `App` remains the shell composition surface
//! during the #1257 phased migration.

pub mod history_paging;
pub mod turn_recovery;
