//! Explicit `kill_container` (#1369 slice 2, #1939).
//!
//! The order is the policy:
//!
//! 1. resolve the target and refuse — before any claim — a record the
//!    caller's [`ClaimCondition`] does not admit (another creator), then an
//!    environment whose script set retained no `kill`;
//! 2. take the registry's exclusive kill claim (no double kill, no kill
//!    racing a final-member cleanup: whichever claims first owns the end),
//!    the condition judged again with the claim itself;
//! 3. ask every member to shut down through the capability's member-shutdown
//!    port (protocol over each member's edge; the owned-handle fallback only
//!    for handles this session owns);
//! 4. only when every member settled, run the retained `kill` exactly once
//!    and commit `stopped` on its success.
//!
//! A `stopped` environment (#2206) has no members and its retained kill
//! already ran — or its container was found gone — but its container and
//! state directory may still be on disk. Its kill is a removal instead:
//! the runtime must affirm, through the retained `inspect`, that its
//! container is not running (a record wrongly `stopped` while another
//! session still runs the box is refused, nothing touched); then, under the
//! same exclusive claim, the retained `cleanup` runs once and the record is
//! forgotten only after it succeeded. The claim is taken before the runtime
//! is asked, and released untouched on a refusal. A removal whose cleanup
//! failed stays owed: the next kill retries the removal, never the kill.
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

use crate::domain::environments::entities::environment_registry::{
    ClaimCondition, ClaimRefusal, CreatorMismatch, EnvironmentLookupError, EnvironmentRecord,
    EnvironmentRegistry, EnvironmentTarget,
};

use super::super::dto::EnvironmentLiveness;

use super::super::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, HostedSwarmRunObservation,
    MemberShutdownReport,
};
use crate::domain::environments::services::environment_retention::SwarmRunObservation;

pub struct KillEnvironment {
    registry: EnvironmentRegistry,
    members: Arc<dyn EnvironmentMemberShutdown>,
    commands: Arc<dyn EnvironmentProcessCommands>,
    /// What a stopped environment's checkout hosts, judged before its
    /// leftovers are removed (#2206 round 4, the collector's own rule).
    hosted: Arc<dyn HostedSwarmRunObservation>,
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
    /// A `stopped` environment's leftovers were removed and its record
    /// forgotten (#2206), rather than a live environment killed.
    pub removed_stopped: bool,
}

/// Every error names the state the registry was left in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillEnvironmentError {
    /// Nothing is there to kill (refused before any claim or effect): an
    /// unknown target, one forgotten before its claim could be taken, or a
    /// `stopped` environment whose script set retained no `cleanup` — what
    /// it left is the collector's. The account says which.
    NothingToKill(String),
    /// Refused before any claim or effect: an ambiguous target, an
    /// already-claimed environment, a script set without a retained `kill`
    /// (or, for a `stopped` one, without a retained `cleanup`), or a
    /// `stopped` one whose container the runtime does not report gone. The
    /// environment is as it was.
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
    /// The kill was conditional on the record's creator
    /// ([`ClaimCondition::CreatedBy`]) and the record under the target was
    /// created by another key (empty for a session-less run). Refused
    /// before or with the claim: nothing was asked, run or changed.
    NotCreatedBy(CreatorMismatch),
}

impl fmt::Display for KillEnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(detail) | Self::NothingToKill(detail) => f.write_str(detail),
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
            Self::NotCreatedBy(CreatorMismatch {
                environment_ref,
                expected,
                created_by,
            }) => {
                let created_by = match created_by.as_str() {
                    "" => "no session",
                    named => named,
                };
                write!(
                    f,
                    "environment {environment_ref} was created by {created_by}, not {expected}; nothing was killed"
                )
            }
        }
    }
}

impl std::error::Error for KillEnvironmentError {}

/// The kill's error for a failed lookup: an unknown target, or a name only
/// stopped environments carry, is nothing to kill; every other failure
/// (ambiguous, stale, unreadable) is a refusal.
fn lookup_refused(error: EnvironmentLookupError) -> KillEnvironmentError {
    match error {
        EnvironmentLookupError::Unknown(_) => {
            KillEnvironmentError::NothingToKill(error.to_string())
        }
        EnvironmentLookupError::Stopped(_) => KillEnvironmentError::NothingToKill(format!(
            "{error}; nothing live has that name (`quecto container ls --all` lists its ref)"
        )),
        EnvironmentLookupError::Ambiguous(_)
        | EnvironmentLookupError::Stale(_)
        | EnvironmentLookupError::Unreadable(_) => KillEnvironmentError::Refused(error.to_string()),
    }
}

/// The kill's error for a refused conditional claim.
fn claim_refused(refusal: ClaimRefusal) -> KillEnvironmentError {
    match refusal {
        ClaimRefusal::Lookup(error) => lookup_refused(error),
        ClaimRefusal::CreatedByOther(mismatch) => KillEnvironmentError::NotCreatedBy(mismatch),
    }
}

/// How a kill of a removable environment went (#2206).
enum Removal {
    /// Its leftovers were removed and its record forgotten.
    Removed(KilledEnvironment),
    /// An owed removal whose container runs again: the ordinary kill ends
    /// it instead.
    OrdinaryKill(EnvironmentRecord),
}

impl KillEnvironment {
    pub fn new(
        registry: EnvironmentRegistry,
        members: Arc<dyn EnvironmentMemberShutdown>,
        commands: Arc<dyn EnvironmentProcessCommands>,
        hosted: Arc<dyn HostedSwarmRunObservation>,
    ) -> Self {
        Self {
            registry,
            members,
            commands,
            hosted,
        }
    }

    pub async fn kill_container(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<KilledEnvironment, KillEnvironmentError> {
        self.kill_container_if(target, &ClaimCondition::Any).await
    }

    /// [`Self::kill_container`] of a record `condition` admits. The
    /// condition is judged on the resolved record first (so a mismatch is
    /// reported before any other refusal) and again by the registry with
    /// the claim itself, so a ref reused in between is never killed.
    pub async fn kill_container_if(
        &self,
        target: &EnvironmentTarget,
        condition: &ClaimCondition,
    ) -> Result<KilledEnvironment, KillEnvironmentError> {
        let mut resolved = self.registry.resolve(target).map_err(lookup_refused)?;
        condition
            .judge(&resolved)
            .map_err(KillEnvironmentError::NotCreatedBy)?;
        if resolved.removable() {
            match self.remove_stopped(resolved, condition).await? {
                Removal::Removed(killed) => return Ok(killed),
                // An owed removal whose container runs again: its end is
                // the ordinary kill's (#2206 round 3).
                Removal::OrdinaryKill(record) => resolved = record,
            }
        }
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
            .begin_kill_if(&resolved.environment_ref, condition)
            .map_err(claim_refused)?;
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
        debug_assert!(
            condition.admits(&record),
            "the claim was granted on the condition"
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
                Ok(KilledEnvironment {
                    record,
                    members,
                    removed_stopped: false,
                })
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

    /// Remove a `stopped` environment's leftovers (#2206). Refused, with no
    /// claim and no effect, unless its script set retained a `cleanup` and
    /// the runtime affirms its container is gone.
    async fn remove_stopped(
        &self,
        record: EnvironmentRecord,
        condition: &ClaimCondition,
    ) -> Result<Removal, KillEnvironmentError> {
        debug_assert!(record.removable());
        debug_assert!(record.members.is_empty(), "a stopped record has no members");
        let environment_ref = record.environment_ref.clone();
        if record.retained_cleanup_argv.is_empty() {
            return Err(KillEnvironmentError::NothingToKill(format!(
                "environment {environment_ref} is stopped and its script set retained no cleanup; `quecto container gc` collects what it left"
            )));
        }
        // Claimed before the runtime is asked, so no kill or other removal
        // acts on the record between the answer and the cleanup.
        let claim = self
            .registry
            .begin_removal_if(&environment_ref, condition)
            .map_err(claim_refused)?;
        let refusal = match self.commands.observe_liveness(&record).await {
            EnvironmentLiveness::Gone => None,
            EnvironmentLiveness::Running if record.removal_pending() => {
                self.registry.abandon_removal(claim);
                let record = self.registry.get(&environment_ref).ok_or_else(|| {
                    KillEnvironmentError::Refused(format!(
                        "environment {environment_ref} left the registry while it was being judged"
                    ))
                })?;
                return Ok(Removal::OrdinaryKill(record));
            }
            EnvironmentLiveness::Running => Some(format!(
                "environment {environment_ref} is stopped in the registry, but its container is running — another session may still use it; nothing was removed"
            )),
            EnvironmentLiveness::Unknown(reason) => Some(format!(
                "environment {environment_ref} is stopped, but whether its container still runs could not be checked ({reason}); nothing was removed"
            )),
        };
        // The collector's own rule (#2206 round 4): a stopped box whose
        // checkout hosts a swarm run its owner has not closed — or a board
        // that cannot be read — is the run's, not leftovers.
        let refusal = match refusal {
            Some(refusal) => Some(refusal),
            None => self.unfinished_run(&record).await,
        };
        if let Some(refusal) = refusal {
            self.registry.release_removal(claim);
            return Err(KillEnvironmentError::Refused(refusal));
        }
        match self
            .commands
            .run_retained_cleanup(&record.environment_id, &record.retained_cleanup_argv)
            .await
        {
            Ok(()) => {
                self.registry.complete_removal(claim);
                Ok(Removal::Removed(KilledEnvironment {
                    record,
                    members: MemberShutdownReport::default(),
                    removed_stopped: true,
                }))
            }
            Err(detail) => {
                self.registry.fail_removal(claim, &detail);
                Err(KillEnvironmentError::KillFailed {
                    environment_ref,
                    detail,
                })
            }
        }
    }

    /// Why a removable environment's leftovers are not leftovers: its
    /// checkout hosts a created swarm run its owner has not closed, or a
    /// board that cannot be read. `None` for no board, the placeholder, a
    /// closed run, or a workspace the host cannot see (nothing to read —
    /// the explicit kill is the owner's word, as for a retained box).
    async fn unfinished_run(&self, record: &EnvironmentRecord) -> Option<String> {
        let environment_ref = &record.environment_ref;
        match self.hosted.observe_hosted_swarm_run(record).await {
            SwarmRunObservation::Run(run) if run.keeps_environment() => Some(format!(
                "environment {environment_ref} is stopped, but its checkout hosts unfinished swarm run {} ({}); close the run first (`swarm_control close` from its supervisor), then kill it again; nothing was removed",
                run.id,
                run.describe()
            )),
            SwarmRunObservation::Unreadable(reason) => Some(format!(
                "environment {environment_ref} is stopped, but its checkout's swarm board could not be read ({reason}); a live run may still need it, so nothing was removed — remove it by hand once you know it is done"
            )),
            SwarmRunObservation::Run(_)
            | SwarmRunObservation::NoStore
            | SwarmRunObservation::NoStoreUnverified => None,
        }
    }
}
