//! The concrete graph of the environments capability (#1369, #1939): the
//! inventory query and the environment kill `agent_cmd` serves, over the
//! session's environment registry, the script command adapters and the
//! member shutdown composition binds; and the final-member cleanup a
//! compensation runs for each membership it removes. Built once per
//! harness beside its agent-control tools and installed in the slots the
//! built tools read.
use std::sync::Arc;

use crate::application::environments::ports::EnvironmentMemberShutdown;
use crate::application::environments::use_cases::{
    FinalizeEnvironmentMember, KillEnvironment, ListEnvironmentsQuery,
};
use crate::domain::environment_registry::EnvironmentRegistry;
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
