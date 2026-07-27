//! Workspace ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns files autocomplete coordination, Git branch footer context, and the
//! workspace filesystem enumeration adapter. Feature-owned App extensions are composed by `shell::app`.

pub mod workspace_files;
