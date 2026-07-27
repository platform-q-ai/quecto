//! Workflow ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns workflow projection controls (auto-continue / completion-nudge mirrors).
//! App slices remain mounted inside `shell::app` until later
//! controller-extraction phases.

// Shell composes these feature-owned flow types as App extensions.
