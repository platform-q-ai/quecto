//! Sessions ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns resume/new/clear/stats presentation flow state. Feature-owned App extensions are composed by `shell::app`.

// Shell composes these feature-owned flow types as App extensions.

#[cfg(test)]
#[path = "resume_picker_r2_tests.rs"]
mod resume_picker_r2_tests;
#[cfg(test)]
#[path = "resume_picker_settle_tests.rs"]
mod resume_picker_settle_tests;
#[cfg(test)]
#[path = "resume_picker_tests.rs"]
mod resume_picker_tests;

pub mod clock;
pub mod discovery_diagnostics;
pub mod local_filter;
pub mod resume_decision;
pub mod resume_picker;
pub mod resume_rows;
pub mod session_search;

#[cfg(test)]
#[path = "resume_rows_tests.rs"]
mod resume_rows_tests;
