//! Workspace ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns files autocomplete coordination, Git branch footer context, and the
//! workspace filesystem enumeration adapter. App slices remain mounted inside
//! `shell::app` until later controller-extraction phases.

pub mod workspace_files;
