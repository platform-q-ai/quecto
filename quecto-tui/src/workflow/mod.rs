//! Workflow ownership for `quecto-tui` (#1257 Phase 5 / Phase 6).
//!
//! Owns workflow projection controls (auto-continue / completion-nudge mirrors).
//! Feature-owned controllers live here; `shell::app` composes them as App
//! extensions without taking ownership of workflow policy.

// Shell composes these feature-owned flow types as App extensions.
