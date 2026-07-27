//! Workflow ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns workflow projection controls (auto-continue / completion-nudge mirrors).
//! App slices remain mounted inside `shell::app` until later
//! controller-extraction phases.

// Flow types are mounted via `#[path]` from `shell::app` so they remain
// sibling modules of `App` during the phased migration.
