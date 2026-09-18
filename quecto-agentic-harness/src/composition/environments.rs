//! The concrete graph of the environments capability (#1369, #1939): the
//! inventory query and the environment kill `agent_cmd` serves, over the
//! session's environment registry, the script command adapters and the
//! member shutdown composition binds; and the final-member cleanup a
//! compensation runs for each membership it removes. Built once per
//! harness beside its agent-control tools and installed in the slots the
//! built tools read. And the container-runtime doctor (#2024 S4b): the
//! create preflight of the effective container config, resolved through
//! the same launch-policy selection a spawn uses.
use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::ConfigSelection;
use crate::application::environments::ports::{
    ContainerConfigLookup, ContainerRuntimePreflight, EnvironmentMemberShutdown,
};
use crate::application::environments::use_cases::{
    DiagnoseContainerRuntime, FinalizeEnvironmentMember, KillEnvironment, ListEnvironmentsQuery,
};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::infrastructure::config::container_config_lookup::SelectedConfigLookup;
use crate::infrastructure::processes::containers::preflight::ScriptPreflight;
pub use crate::infrastructure::tools::agent_cmd_containers::EnvironmentControl;
use crate::infrastructure::tools::environment_commands::{
    HostedStoreObservation, ScriptEnvironmentCommands,
};

/// The final-member use case over the production script adapters, for one
/// entry's environment registry. The cleanup jobs already run on a blocking
/// worker, so the scripts run inline.
pub fn build_member_finalizer(environments: EnvironmentRegistry) -> FinalizeEnvironmentMember {
    FinalizeEnvironmentMember::new(
        environments,
        Arc::new(ScriptEnvironmentCommands::inline()),
        Arc::new(HostedStoreObservation),
    )
}

/// The environment control `agent_cmd` invokes: the side-effect-free
/// listing and the kill that asks the environment's members to shut down
/// through `member_shutdown` before its retained kill.
pub fn build_environment_control(
    environments: EnvironmentRegistry,
    member_shutdown: Arc<dyn EnvironmentMemberShutdown>,
) -> EnvironmentControl {
    EnvironmentControl {
        list: Arc::new(ListEnvironmentsQuery::new(environments.clone())),
        kill: Arc::new(KillEnvironment::new(
            environments,
            member_shutdown,
            Arc::new(ScriptEnvironmentCommands::default()),
        )),
    }
}

/// The doctor `quecto container doctor` invokes (#2024 S4b), over the
/// run's own configuration selection — the layers resolved for its
/// working directory, or the explicit `--config` file — with trust
/// recorded under `base_dir`. `main` hands this builder to the CLI entry
/// point; the interface never composes the selection or runs a script.
pub fn build_container_doctor(
    base_dir: &Path,
    selection: &ConfigSelection,
) -> Arc<DiagnoseContainerRuntime> {
    Arc::new(DiagnoseContainerRuntime::new(
        build_container_config_lookup(base_dir, Some(selection.clone())),
        build_container_runtime_preflight(),
    ))
}

/// The lookup port adapter alone, for the contract suite: the doctor's
/// target resolved through the launch policy's selection.
pub fn build_container_config_lookup(
    base_dir: &Path,
    selection: Option<ConfigSelection>,
) -> Arc<dyn ContainerConfigLookup> {
    Arc::new(SelectedConfigLookup::new(
        super::container_configs::build_container_config_selection(base_dir, selection),
    ))
}

/// The preflight port adapter alone, for the contract suite.
pub fn build_container_runtime_preflight() -> Arc<dyn ContainerRuntimePreflight> {
    Arc::new(ScriptPreflight)
}
