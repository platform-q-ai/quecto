//! Session environment use cases.

mod diagnose_container_runtime;
mod finalize_environment_member;
mod gc_orphaned_environments;
mod kill_environment;
mod list_environments;
mod restore_registry;

pub use diagnose_container_runtime::DiagnoseContainerRuntime;
pub use finalize_environment_member::FinalizeEnvironmentMember;
pub use gc_orphaned_environments::{GcOrphanedEnvironments, implied_state_root};
pub use kill_environment::{KillEnvironment, KillEnvironmentError, KilledEnvironment};
pub use list_environments::ListEnvironmentsQuery;
pub use restore_registry::{GONE_AT_RESTORE, KILL_INTERRUPTED, RestoreRegistry};

#[cfg(test)]
mod finalize_environment_member_retention_tests;
#[cfg(test)]
mod finalize_environment_member_tests;
#[cfg(test)]
mod kill_environment_tests;
#[cfg(test)]
mod list_environments_tests;
