//! Compatibility re-export of the text-input system [`Editor`].
//!
//! Prefer `crate::components::text_input::Editor`. This path remains so existing
//! call sites (`components::editor::Editor`) keep compiling without parallel
//! implementations.

pub use crate::components::text_input::Editor;
