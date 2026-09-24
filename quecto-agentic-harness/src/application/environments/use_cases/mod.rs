//! Session environment use cases.

mod box_residue;
mod container_status;
mod diagnose_container_runtime;
mod finalize_environment_member;
mod gc_orphaned_environments;
mod initialise_standard_container;
mod kill_environment;
mod list_container_configs;
mod list_environments;
mod restore_registry;

pub use box_residue::{NO_ENVIRONMENT_DIR, RESIDUE_INSPECT_BUDGET, STATE_GONE_CONTAINER_RUNNING};
pub use container_status::ContainerStatus;
pub use diagnose_container_runtime::DiagnoseContainerRuntime;
pub use finalize_environment_member::FinalizeEnvironmentMember;
pub use gc_orphaned_environments::{
    CREATE_GRACE_SECS, GcOrphanedEnvironments, implied_state_root, retained_state_root,
};
pub use initialise_standard_container::{InitialiseStandardContainer, STATE_DIR_NAME};
pub use kill_environment::{KillEnvironment, KillEnvironmentError, KilledEnvironment};
pub use list_container_configs::ListContainerConfigs;
pub use list_environments::ListEnvironmentsQuery;
pub use restore_registry::{
    KILL_IN_FLIGHT, RELABELLED_BY_OLDER_BUILD, RETAINED_EXITED, RestoreRegistry,
    unfinished_run_reason,
};

#[cfg(test)]
mod finalize_environment_member_retention_tests;
#[cfg(test)]
mod finalize_environment_member_tests;
#[cfg(test)]
mod gc_orphaned_environments_abandoned_tests;
#[cfg(test)]
mod gc_orphaned_environments_hosted_tests;
#[cfg(test)]
mod gc_orphaned_environments_scope_tests;
#[cfg(test)]
mod gc_orphaned_environments_tests;
#[cfg(test)]
mod kill_environment_tests;
#[cfg(test)]
mod list_container_configs_tests;
#[cfg(test)]
mod list_environments_tests;
#[cfg(test)]
mod restore_registry_forget_tests;
#[cfg(test)]
mod restore_registry_hosted_tests;
#[cfg(test)]
mod restore_registry_tests;
