//! Sessions ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns resume/new/clear/stats presentation flow state. Feature-owned App extensions are composed by `shell::app`.

// Shell composes these feature-owned flow types as App extensions.

#[cfg(test)]
#[path = "resume_picker_tests.rs"]
mod resume_picker_tests;

pub mod resume_picker;
