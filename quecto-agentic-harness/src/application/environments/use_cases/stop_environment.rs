//! Explicit non-destructive runtime stop for retained environments.
//!
//! The runtime is retired while its state directory, checkout and coordination
//! board remain available for data recovery. Eligibility is deliberately an
//! affirmative allowlist: only an environment already marked `Retained` may be
//! stopped. Successful completion produces the non-joinable `Preserved` state;
//! destructive removal remains the separate `kill_container` use case.

use std::fmt;
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentTarget,
};

use super::super::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport,
};

pub struct StopEnvironment {
    registry: EnvironmentRegistry,
    members: Arc<dyn EnvironmentMemberShutdown>,
    commands: Arc<dyn EnvironmentProcessCommands>,
}

impl fmt::Debug for StopEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StopEnvironment")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedEnvironment {
    pub record: EnvironmentRecord,
    pub members: MemberShutdownReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopEnvironmentError {
    Refused(String),
    MembersUnsettled {
        environment_ref: String,
        members: Vec<(String, String)>,
    },
    StopFailed {
        environment_ref: String,
        detail: String,
    },
}

impl fmt::Display for StopEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(detail) => f.write_str(detail),
            Self::MembersUnsettled {
                environment_ref,
                members,
            } => {
                let listed: Vec<String> = members
                    .iter()
                    .map(|(member, detail)| format!("{member} ({detail})"))
                    .collect();
                write!(
                    f,
                    "environment {environment_ref} stop refused: members not settled: {}; workspace remains retained, retry stop_container",
                    listed.join(", ")
                )
            }
            Self::StopFailed {
                environment_ref,
                detail,
            } => write!(
                f,
                "environment {environment_ref} runtime stop failed: {detail}; workspace remains retained, retry stop_container"
            ),
        }
    }
}

impl std::error::Error for StopEnvironmentError {}

impl StopEnvironment {
    pub fn new(
        registry: EnvironmentRegistry,
        members: Arc<dyn EnvironmentMemberShutdown>,
        commands: Arc<dyn EnvironmentProcessCommands>,
    ) -> Self {
        Self {
            registry,
            members,
            commands,
        }
    }

    pub async fn stop_container(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<PreservedEnvironment, StopEnvironmentError> {
        let resolved = self
            .registry
            .resolve(target)
            .map_err(|error| StopEnvironmentError::Refused(error.to_string()))?;
        if resolved.retained_kill_argv.is_empty() {
            return Err(StopEnvironmentError::Refused(format!(
                "environment {} has no retained control argv; its runtime does not support stop_container",
                resolved.environment_ref
            )));
        }
        let claim = self
            .registry
            .begin_stop(&resolved.environment_ref)
            .map_err(|error| StopEnvironmentError::Refused(error.to_string()))?;
        let record = self
            .registry
            .get(&resolved.environment_ref)
            .unwrap_or(resolved);
        debug_assert!(
            !record.retained_kill_argv.is_empty(),
            "the retained runtime control argv is immutable once committed"
        );

        let members = self.members.shutdown_members(&record.members).await;
        if !members.all_settled() {
            let unsettled = members
                .unsettled
                .iter()
                .map(|member| (member.member.clone(), member.detail.clone()))
                .collect();
            self.registry.fail_stop(
                claim,
                "members did not settle before the non-destructive runtime stop",
            );
            return Err(StopEnvironmentError::MembersUnsettled {
                environment_ref: record.environment_ref,
                members: unsettled,
            });
        }

        match self
            .commands
            .run_retained_stop(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => {
                self.registry.complete_stop(claim);
                Ok(PreservedEnvironment { record, members })
            }
            Err(detail) => {
                self.registry.fail_stop(claim, &detail);
                Err(StopEnvironmentError::StopFailed {
                    environment_ref: record.environment_ref,
                    detail,
                })
            }
        }
    }
}
