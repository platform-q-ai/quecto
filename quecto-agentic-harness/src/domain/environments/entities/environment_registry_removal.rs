//! Removal of a `stopped` environment's leftovers (#2206), a child of
//! [`super`] so it reaches the registry's state. A `stopped` record whose
//! container has exited may still hold its state directory (the restore
//! marked it stopped when it found the container gone); an explicit
//! `container kill` removes what is left through the retained `cleanup`
//! and then forgets the record. The claim is the same exclusive kill
//! claim, so a removal never races a kill or another removal. A removal
//! whose cleanup failed is marked owed, so the next kill retries it.

use super::{
    EnvironmentLookupError, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, KillClaim,
};

/// The `metadata` key a removal whose cleanup failed leaves on the record
/// (#2206): the environment was being removed for good, so the next kill
/// retries that removal — cleanup and forget — never the ordinary kill.
pub const REMOVAL_PENDING: &str = "removal_pending";

impl EnvironmentRecord {
    /// A removal of this environment failed and is still owed.
    pub fn removal_pending(&self) -> bool {
        self.status == EnvironmentStatus::CleanupFailed
            && self.metadata.get(REMOVAL_PENDING) == Some(&serde_json::Value::Bool(true))
    }

    /// Whether a kill of this environment is a removal of what it left:
    /// it is plainly `stopped` (not an older build's relabel of a retained
    /// box), or an earlier removal of it failed.
    pub fn removable(&self) -> bool {
        self.is_plain_stopped() || self.removal_pending()
    }
}

impl EnvironmentRecord {
    /// Drop the owed-removal mark (#2206 round 4): a record that is plainly
    /// `stopped` again — a restore found its container gone, or a removal
    /// claims it afresh — owes nothing, so a stale mark must never turn a
    /// later refusal into an owed removal.
    pub fn clear_removal_pending(&mut self) {
        if let Some(metadata) = self.metadata.as_object_mut() {
            metadata.remove(REMOVAL_PENDING);
        }
    }
}

/// The exclusive claim on a removal, carrying the status it was claimed
/// from (#2206 round 4): a refused removal returns the record to exactly
/// that status, never one inferred from its metadata.
#[derive(Debug)]
pub struct RemovalClaim {
    claim: KillClaim,
    claimed_from: EnvironmentStatus,
}

impl RemovalClaim {
    /// The status the record held when the removal claimed it.
    pub fn claimed_from(&self) -> &EnvironmentStatus {
        &self.claimed_from
    }
}

impl EnvironmentRegistry {
    /// Claim the removal of a `stopped` environment, or the retry of a
    /// removal that failed: the record moves to `killing` under this
    /// session's claim, which remembers the status it came from. A plainly
    /// stopped record's stale owed-removal mark is dropped. Every other
    /// state keeps its own kill path.
    pub fn begin_removal(
        &self,
        environment_ref: &str,
    ) -> Result<RemovalClaim, EnvironmentLookupError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .entries
            .get_mut(environment_ref)
            .ok_or_else(|| EnvironmentLookupError::Unknown(environment_ref.to_string()))?;
        if record.removable() {
            let claimed_from = record.status.clone();
            if record.is_plain_stopped() {
                record.clear_removal_pending();
            }
            record.status = EnvironmentStatus::Killing;
            let claim = KillClaim {
                environment_ref: record.environment_ref.clone(),
            };
            state.kill_claims.insert(environment_ref.to_string());
            drop(state);
            self.journal_ref(environment_ref);
            return Ok(RemovalClaim {
                claim,
                claimed_from,
            });
        }
        Err(EnvironmentLookupError::Stale(
            record.environment_ref.clone(),
        ))
    }

    /// The removal was refused after its claim (the runtime could not
    /// affirm the container gone, or its board holds an unfinished run):
    /// the record returns to exactly the status it was claimed from.
    pub fn release_removal(&self, claim: RemovalClaim) {
        let RemovalClaim {
            claim,
            claimed_from,
        } = claim;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.kill_claims.remove(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            debug_assert_eq!(record.status, EnvironmentStatus::Killing);
            record.status = claimed_from;
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// The removal's cleanup failed: `cleanup-failed` with the script's
    /// account, and the removal marked owed so the next kill retries it.
    pub fn fail_removal(&self, claim: RemovalClaim, error: &str) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(record) = state.entries.get_mut(&claim.claim.environment_ref) {
                match record.metadata.as_object_mut() {
                    Some(metadata) => {
                        metadata.insert(REMOVAL_PENDING.to_string(), serde_json::Value::Bool(true));
                    }
                    // Create metadata is an object by contract; anything
                    // else carries no key to keep.
                    None => record.metadata = serde_json::json!({ REMOVAL_PENDING: true }),
                }
            }
        }
        self.fail_kill(claim.claim, error);
    }

    /// The owed removal found its container running again (#2206 round 3):
    /// the removal is no longer owed — the box is live, so its end is the
    /// ordinary kill's. The record returns to `cleanup-failed`, unmarked,
    /// where the ordinary kill claims it.
    pub fn abandon_removal(&self, claim: RemovalClaim) {
        debug_assert_eq!(claim.claimed_from, EnvironmentStatus::CleanupFailed);
        let claim = claim.claim;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.kill_claims.remove(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            debug_assert_eq!(record.status, EnvironmentStatus::Killing);
            record.clear_removal_pending();
            record.status = EnvironmentStatus::CleanupFailed;
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// The removal's cleanup succeeded: nothing of the environment is left,
    /// so its record is forgotten (the durable registry with it).
    pub fn complete_removal(&self, claim: RemovalClaim) {
        let removed = self.remove(&claim.claim.environment_ref);
        debug_assert!(
            removed.is_none_or(|record| record.status == EnvironmentStatus::Killing),
            "a removal forgets only the record it claimed"
        );
    }
}
