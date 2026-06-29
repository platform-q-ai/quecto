//! Compatibility re-export for extension tool invocation requests.
//!
//! The concrete type lives in the domain layer so infrastructure and interface
//! code can both depend inward without crossing architecture boundaries.

pub use crate::domain::extension_tool::ToolInvocation;
