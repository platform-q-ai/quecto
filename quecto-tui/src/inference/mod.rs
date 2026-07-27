//! Inference ownership for `quecto-tui` (#1257 Phase 5).
//!
//! Owns model and effort presentation flows (registry, selectors, command
//! routing). App slices remain mounted inside `interface::app` until later
//! controller-extraction phases.

// Flow types are mounted via `#[path]` from `interface::app` so they remain
// sibling modules of `App` during the phased migration.
