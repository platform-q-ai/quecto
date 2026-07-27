//! Inference ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns model and effort presentation flows (registry, selectors, command
//! routing). App slices remain mounted inside `shell::app` until later
//! controller-extraction phases.

// Shell composes these feature-owned flow types as App extensions.
