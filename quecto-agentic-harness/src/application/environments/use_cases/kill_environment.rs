//! Explicit `kill_container` (#1369 slice 2, #1939).
//!
//! The order is the policy:
//!
//! 1. resolve the target and refuse — before any claim — an environment
//!    whose script set retained no `kill`;
//! 2. take the registry's exclusive kill claim (no double kill, no kill
//!    racing a final-member cleanup: whichever claims first owns the end);
//! 3. ask every member to shut down through the capability's member-shutdown
//!    port (protocol over each member's edge; the owned-handle fallback only
//!    for handles this session owns);
//! 4. only when every member settled, run the retained `kill` exactly once
//!    and commit `stopped` on its success.
//!
//! Anything else leaves a truthful, retryable `cleanup-failed` state under
//! the claim: an unsettled member (a termination still executing on it did
//! not settle within the bound, or its own fallback could not end it, its
//! claim lifted) or a failed kill script. A member whose earlier kill
//! *returned* after effects without observing the end (its claim kept for
//! the exit, its owner gone) is not unsettled forever: the member shutdown
//! re-takes that claim, asks again and — for a member this session holds
//! no handle for — compensates it `unobserved` once the exit does not
//! arrive within the bound, so the retained kill, the box's real
//! authority, runs. The environment is never reported stopped while the
//! retained kill has not succeeded, and the retained kill never runs twice
//! under one claim.
use std::fmt;
use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentTarget,
};

use super::super::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport,
};

pub struct KillEnvironment {
    registry: EnvironmentRegistry,
    members: Arc<dyn EnvironmentMemberShutdown>,
    commands: Arc<dyn EnvironmentProcessCommands>,
}

impl fmt::Debug for KillEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KillEnvironment")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

/// The environment as of the moment the claim was granted, and how each of
/// its members was settled before the retained kill ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KilledEnvironment {
    pub record: EnvironmentRecord,
    pub members: MemberShutdownReport,
}

/// Every error names the state the registry was left in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillEnvironmentError {
    /// Refused before any claim or effect: unknown/ambiguous target, a
    /// stopped or already-claimed environment, or a script set without a
    /// retained `kill`. The environment is as it was.
    Refused(String),
    /// Members whose end could not be settled; the environment is
    /// `cleanup-failed` with its claim released for a retry.
    MembersUnsettled {
        environment_ref: String,
        members: Vec<(String, String)>,
    },
    /// The retained kill ran once and reported failure; the environment is
    /// `cleanup-failed` with its claim released for a retry.
    KillFailed {
        environment_ref: String,
        detail: String,
    },
}

impl fmt::Display for KillEnvironmentError {
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
                    "environment {environment_ref} cleanup failed: members not settled: {}; state is cleanup-failed, retry kill_container",
                    listed.join(", ")
                )
            }
            Self::KillFailed {
                environment_ref,
                detail,
            } => write!(
                f,
                "environment {environment_ref} cleanup failed: {detail}; state is cleanup-failed, retry kill_container"
            ),
        }
    }
}

impl std::error::Error for KillEnvironmentError {}

impl KillEnvironment {
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

    /// Preserve a retained environment's data while retiring its runtime.
    /// Kept on the same composed control owner as kill so existing sessions
    /// gain the capability without a second mutable composition slot.
    pub async fn stop_container(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<super::PreservedEnvironment, super::StopEnvironmentError> {
        super::StopEnvironment::new(
            self.registry.clone(),
            self.members.clone(),
            self.commands.clone(),
        )
        .stop_container(target)
        .await
    }

    pub async fn kill_container(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<KilledEnvironment, KillEnvironmentError> {
        let resolved = self
            .registry
            .resolve(target)
            .map_err(|e| KillEnvironmentError::Refused(e.to_string()))?;
        // Refuse before claiming: a script set with no `kill` must leave the
        // environment Running and its members untouched, so joins keep
        // working and final-member exit still runs the retained cleanup
        // fallback.
        if resolved.retained_kill_argv.is_empty() {
            return Err(KillEnvironmentError::Refused(format!(
                "environment {} has no retained kill argv; its script set does not support kill_container",
                resolved.environment_ref
            )));
        }
        let claim = self
            .registry
            .begin_kill(&resolved.environment_ref)
            .map_err(|e| KillEnvironmentError::Refused(e.to_string()))?;
        // Re-read under the claim so the kill sees the members as of the
        // moment the claim was granted.
        let record = self
            .registry
            .get(&resolved.environment_ref)
            .unwrap_or(resolved);
        debug_assert!(
            !record.retained_kill_argv.is_empty(),
            "the retained kill argv is immutable once committed"
        );

        let members = self.members.shutdown_members(&record.members).await;
        if !members.all_settled() {
            let unsettled: Vec<(String, String)> = members
                .unsettled
                .iter()
                .map(|m| (m.member.clone(), m.detail.clone()))
                .collect();
            let error = KillEnvironmentError::MembersUnsettled {
                environment_ref: record.environment_ref.clone(),
                members: unsettled,
            };
            self.registry.fail_kill(claim, &error.to_string());
            return Err(error);
        }

        match self
            .commands
            .run_retained_kill(&record.environment_id, &record.retained_kill_argv)
            .await
        {
            Ok(()) => {
                self.registry.complete_kill(claim);
                Ok(KilledEnvironment { record, members })
            }
            Err(detail) => {
                self.registry.fail_kill(claim, &detail);
                Err(KillEnvironmentError::KillFailed {
                    environment_ref: record.environment_ref,
                    detail,
                })
            }
        }
    }
}
