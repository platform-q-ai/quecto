//! Session-scoped registry of script-managed environments.
//!
//! Per ADR-0021 composition builds exactly one registry per session and
//! injects it into the launch services. It is the authority for minting
//! never-reused `C1`-style environment refs and for recording which
//! environments this session has committed: hidden environment UUID, optional
//! name, script/runtime identity, retained script argv, member agent UUIDs,
//! status, metadata, and last error (#1369 slice 2).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Lifecycle status of one committed environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentStatus {
    /// Live and joinable.
    Running,
    /// A kill claim is outstanding; not joinable, not yet stopped.
    Killing,
    /// Kill succeeded; terminal. The record stays listed, the ref is never reused.
    Stopped,
    /// Kill failed; retryable via another kill, with `last_error` retained.
    CleanupFailed,
}

/// How a caller addresses an existing environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentTarget {
    /// Session-scoped `CN` ref minted by this registry.
    Ref(String),
    /// Optional user-facing environment name; must resolve unambiguously.
    Name(String),
}

/// Resolution failures. Never guesses: unknown, ambiguous, stopped, and stale
/// targets each fail with their own actionable error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentLookupError {
    Unknown(String),
    Ambiguous(String),
    Stopped(String),
    Stale(String),
}

impl std::fmt::Display for EnvironmentLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(t) => write!(f, "environment '{t}' is unknown in this session"),
            Self::Ambiguous(t) => write!(f, "environment name '{t}' is ambiguous in this session"),
            Self::Stopped(t) => write!(f, "environment '{t}' is stopped"),
            Self::Stale(t) => write!(
                f,
                "environment '{t}' is stale: cleanup is pending or failed; retry kill_container"
            ),
        }
    }
}

impl std::error::Error for EnvironmentLookupError {}

/// One committed script-managed environment known to this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRecord {
    /// Session-local, never-reused ref (e.g. `C1`) minted by this registry.
    pub environment_ref: String,
    /// Script/runtime-owned environment identity from the create result.
    pub environment_id: String,
    /// Hidden environment UUID minted by Quecto; distinct from the ref, the
    /// runtime id, and every member agent UUID.
    pub environment_uuid: String,
    /// Optional user-facing environment name.
    pub name: Option<String>,
    /// Workspace path reported by the create result; shared by all members.
    pub workspace_path: PathBuf,
    /// Repository URL the environment was created for.
    pub repository: String,
    /// Name of the configured container script set that created it.
    pub script_name: String,
    /// Exec argv retained at create time; joins use this even if the
    /// configured default script set changes later.
    pub retained_exec_argv: Vec<String>,
    /// Kill argv retained at create time; final-member and explicit cleanup
    /// use this exactly once per successful kill.
    pub retained_kill_argv: Vec<String>,
    /// Member agent UUIDs, in join order.
    pub members: Vec<String>,
    pub status: EnvironmentStatus,
    /// Metadata object from the create result.
    pub metadata: serde_json::Value,
    /// Last cleanup error, retained while status is `CleanupFailed`.
    pub last_error: Option<String>,
}

impl EnvironmentRecord {
    /// User-facing status label: a live environment with no members reads as
    /// `empty`, otherwise the lifecycle status names itself.
    pub fn status_label(&self) -> &'static str {
        match self.status {
            EnvironmentStatus::Running if self.members.is_empty() => "empty",
            EnvironmentStatus::Running => "running",
            EnvironmentStatus::Killing => "killing",
            EnvironmentStatus::Stopped => "stopped",
            EnvironmentStatus::CleanupFailed => "cleanup-failed",
        }
    }
}

/// Mint the hidden environment UUID committed with each new environment.
/// Distinct from the `CN` ref, the runtime id, and agent UUIDs by
/// construction (fresh v4 UUID per environment).
pub fn mint_environment_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Proof that the caller holds the exclusive right to run this environment's
/// kill operation. Only `begin_kill`/`remove_member` hand one out; it must be
/// settled with `complete_kill` or `fail_kill`.
#[derive(Debug)]
pub struct KillClaim {
    environment_ref: String,
}

/// Outcome of removing one member from an environment.
#[derive(Debug)]
pub struct MemberRemoval {
    /// True when this removal made the environment empty and atomically
    /// claimed its final cleanup. Exactly one racer can observe true.
    pub final_member_cleanup_claimed: bool,
    /// The kill claim when `final_member_cleanup_claimed` is true.
    pub claim: Option<KillClaim>,
}

#[derive(Debug, Default)]
struct EnvironmentRegistryState {
    next_ref: u64,
    entries: BTreeMap<String, EnvironmentRecord>,
}

/// Cloneable handle to one session's environment registry.
#[derive(Debug, Clone, Default)]
pub struct EnvironmentRegistry {
    state: Arc<Mutex<EnvironmentRegistryState>>,
}

impl EnvironmentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint the next `CN` ref. Refs are monotonic and never reused within a
    /// session, even when the launch they were minted for later fails or the
    /// environment is stopped.
    pub fn mint_ref(&self) -> String {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.next_ref += 1;
        format!("C{}", state.next_ref)
    }

    /// Commit a created environment under its minted ref.
    pub fn commit(&self, record: EnvironmentRecord) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.insert(record.environment_ref.clone(), record);
    }

    /// Remove a committed environment (launch rollback/uncommit only; a
    /// stopped environment stays listed and its ref is never reused).
    pub fn remove(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.remove(environment_ref)
    }

    pub fn get(&self, environment_ref: &str) -> Option<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.get(environment_ref).cloned()
    }

    pub fn entries(&self) -> Vec<EnvironmentRecord> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.values().cloned().collect()
    }

    /// Resolve a target to its committed record regardless of status.
    pub fn resolve(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::resolve_locked(&state, target)
    }

    /// Resolve a target for joining: the environment must be live. Stopped
    /// environments fail as stopped; killing/cleanup-failed environments fail
    /// as stale. Never guesses.
    pub fn resolve_joinable(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        let record = self.resolve(target)?;
        match record.status {
            EnvironmentStatus::Running => Ok(record),
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
            EnvironmentStatus::Killing | EnvironmentStatus::CleanupFailed => Err(
                EnvironmentLookupError::Stale(record.environment_ref.clone()),
            ),
        }
    }

    fn resolve_locked(
        state: &EnvironmentRegistryState,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, EnvironmentLookupError> {
        match target {
            EnvironmentTarget::Ref(env_ref) => state
                .entries
                .get(env_ref)
                .cloned()
                .ok_or_else(|| EnvironmentLookupError::Unknown(env_ref.clone())),
            EnvironmentTarget::Name(name) => {
                let mut matches = state
                    .entries
                    .values()
                    .filter(|r| r.name.as_deref() == Some(name.as_str()));
                match (matches.next(), matches.next()) {
                    (Some(record), None) => Ok(record.clone()),
                    (Some(_), Some(_)) => Err(EnvironmentLookupError::Ambiguous(name.clone())),
                    (None, _) => Err(EnvironmentLookupError::Unknown(name.clone())),
                }
            }
        }
    }

    /// Record one member agent UUID joining the environment.
    pub fn add_member(
        &self,
        environment_ref: &str,
        agent_uuid: &str,
    ) -> Result<(), EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        if !record.members.iter().any(|m| m == agent_uuid) {
            record.members.push(agent_uuid.to_string());
        }
        Ok(())
    }

    /// Remove one member. When the environment becomes (or already is) empty
    /// and still running, this atomically claims its final cleanup: exactly
    /// one concurrent remover observes `final_member_cleanup_claimed`.
    pub fn remove_member(
        &self,
        environment_ref: &str,
        agent_uuid: &str,
    ) -> Result<MemberRemoval, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        record.members.retain(|m| m != agent_uuid);
        if record.members.is_empty() && record.status == EnvironmentStatus::Running {
            record.status = EnvironmentStatus::Killing;
            Ok(MemberRemoval {
                final_member_cleanup_claimed: true,
                claim: Some(KillClaim {
                    environment_ref: environment_ref.to_string(),
                }),
            })
        } else {
            Ok(MemberRemoval {
                final_member_cleanup_claimed: false,
                claim: None,
            })
        }
    }

    /// Claim the exclusive right to run this environment's kill operation.
    /// Refused while another claim is outstanding (no double-kill) and after a
    /// successful kill; allowed again after a failed kill (retry).
    pub fn begin_kill(&self, environment_ref: &str) -> Result<KillClaim, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        match record.status {
            EnvironmentStatus::Running | EnvironmentStatus::CleanupFailed => {
                record.status = EnvironmentStatus::Killing;
                Ok(KillClaim {
                    environment_ref: record.environment_ref.clone(),
                })
            }
            EnvironmentStatus::Killing => Err(EnvironmentLookupError::Stale(
                record.environment_ref.clone(),
            )),
            EnvironmentStatus::Stopped => Err(EnvironmentLookupError::Stopped(
                record.environment_ref.clone(),
            )),
        }
    }

    /// Commit stopped after a successful kill. Members are gone by definition.
    pub fn complete_kill(&self, claim: KillClaim) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            record.status = EnvironmentStatus::Stopped;
            record.members.clear();
            record.last_error = None;
        }
    }

    /// Persist a retryable cleanup-failed state with an actionable error.
    pub fn fail_kill(&self, claim: KillClaim, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            record.status = EnvironmentStatus::CleanupFailed;
            record.last_error = Some(error.to_string());
        }
    }
}

#[cfg(test)]
#[path = "environment_registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "environment_registry_slice2_tests.rs"]
mod slice2_tests;
