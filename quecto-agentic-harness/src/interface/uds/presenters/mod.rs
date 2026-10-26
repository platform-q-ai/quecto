//! UDS presenters (#2370): application results rendered as wire responses.
pub mod effort_presenter;
pub mod list_models_presenter;
pub mod model_presenter;
pub mod refresh_presenter;
pub mod reload_presenter;
pub mod subagent_info_list;
pub mod subagent_teardown_presenter;
pub(in crate::interface) mod uds_admission_projection;
#[cfg(test)]
mod uds_admission_projection_tests;
pub(in crate::interface) mod uds_admission_warnings;
pub(in crate::interface) mod uds_busy_state;
pub mod uds_execution_state;
#[cfg(test)]
mod uds_execution_state_tests;
pub(in crate::interface) mod uds_latest_report;
pub(in crate::interface) mod uds_sync;
#[cfg(test)]
mod uds_sync_tests;
