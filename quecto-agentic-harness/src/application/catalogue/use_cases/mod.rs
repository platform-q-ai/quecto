//! Use cases of the catalogue capability. Constructed only by
//! `composition::catalogue`; interface holds injected handles.

pub mod change_reasoning_effort;
pub mod list_models;

pub use change_reasoning_effort::ChangeReasoningEffort;
pub use list_models::ListModels;
