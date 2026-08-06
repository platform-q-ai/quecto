//! Application use case for final-member environment cleanup.
//!
//! Membership removal may mint the exclusive final cleanup claim atomically in
//! the domain registry. This use case owns the surrounding application
//! transaction; infrastructure supplies only the retained script executions.

use std::sync::Arc;

use crate::domain::environment_registry::EnvironmentRegistry;
use crate::domain::subagent_launch::LaunchFuture;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberFinalizeMode {
    Exit,
    ParentKill,
    LaunchRollback,
    LaunchRollbackOwned,
}

pub trait EnvironmentFinalizationPort: Send + Sync {
    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>>;

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>>;

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()>;
}

pub struct EnvironmentFinalizationUseCase {
    registry: EnvironmentRegistry,
    port: Arc<dyn EnvironmentFinalizationPort>,
}

impl EnvironmentFinalizationUseCase {
    pub fn new(registry: EnvironmentRegistry, port: Arc<dyn EnvironmentFinalizationPort>) -> Self {
        Self { registry, port }
    }

    pub async fn finalize_member(
        &self,
        env_ref: &str,
        agent_uuid: &str,
        entry_cleanup_plan: Option<(String, Vec<String>)>,
        mode: MemberFinalizeMode,
    ) {
        if mode == MemberFinalizeMode::Exit {
            self.inspect_once(env_ref, agent_uuid).await;
        }

        let Ok(removal) = self.registry.remove_member(env_ref, agent_uuid) else {
            return;
        };
        let Some(claim) = removal else {
            return;
        };
        let Some(record) = self.registry.get(env_ref) else {
            self.registry.complete_kill(claim);
            return;
        };

        let launch_rollback = matches!(
            mode,
            MemberFinalizeMode::LaunchRollback | MemberFinalizeMode::LaunchRollbackOwned
        );
        let run_cleanup = launch_rollback || record.retained_kill_argv.is_empty();
        if run_cleanup {
            if !record.retained_cleanup_argv.is_empty() {
                self.port
                    .run_retained_cleanup(&record.environment_id, &record.retained_cleanup_argv)
                    .await;
            } else if let Some((env_id, argv)) = entry_cleanup_plan {
                self.port.run_retained_cleanup(&env_id, &argv).await;
            }
            self.registry.complete_kill(claim);
            if mode == MemberFinalizeMode::LaunchRollbackOwned {
                self.registry.remove(env_ref);
            }
            return;
        }

        match self
            .port
            .run_retained_kill(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => self.registry.complete_kill(claim),
            Err(e) => self.registry.fail_kill(claim, &e),
        }
    }

    async fn inspect_once(&self, env_ref: &str, agent_uuid: &str) {
        let Some(record) = self.registry.get(env_ref) else {
            return;
        };
        if record.retained_inspect_argv.is_empty() {
            return;
        }
        let Some(claim) = self.registry.begin_inspect(env_ref, agent_uuid) else {
            return;
        };
        match self
            .port
            .run_retained_inspect(&record.environment_id, &record.retained_inspect_argv)
            .await
        {
            Ok(metadata) => self.registry.record_inspect_success(claim, metadata),
            Err(e) => self.registry.record_inspect_failure(claim, &e),
        }
    }
}
