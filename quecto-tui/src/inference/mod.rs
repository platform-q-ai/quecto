//! Inference ownership for `quecto-tui` (#1257 Phase 5 / Phase 6).
//!
//! Owns model and effort presentation flows (registry, selectors, command
//! routing). Feature-owned controllers live here; `shell::app` composes them
//! as App extensions without taking ownership of inference policy.

// Shell composes these feature-owned flow types as App extensions.
