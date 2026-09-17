//! Use cases of the catalogue capability. Constructed only by
//! `composition::catalogue`; interface holds injected handles.

pub mod change_active_model;
pub mod change_reasoning_effort;
pub mod list_models;
pub mod refresh_catalogue_sources;
pub mod reload_runtime_configuration;

pub use change_active_model::ChangeActiveModel;
pub use change_reasoning_effort::ChangeReasoningEffort;
pub use list_models::ListModels;
pub use refresh_catalogue_sources::RefreshCatalogueSources;
pub use reload_runtime_configuration::ReloadRuntimeConfiguration;
