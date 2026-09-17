//! Sessions ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns resume/new/clear/stats presentation flow state. Feature-owned App extensions are composed by `shell::app`.

// Shell composes these feature-owned flow types as App extensions.

#[cfg(test)]
#[path = "resume_picker_tests.rs"]
mod resume_picker_tests;

pub mod discovery_diagnostics;
pub mod resume_picker;
pub mod resume_rows;

#[cfg(test)]
#[path = "resume_rows_tests.rs"]
mod resume_rows_tests;
