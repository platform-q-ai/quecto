//! The post-mortem inspect claims of the environment registry (#1369
//! slice 3), a child of [`super`] so it reaches the registry's state: one
//! claim per dead member, its outcome merged onto the record and
//! journalled like every other change.

use super::{EnvironmentRegistry, EnvironmentStatus, InspectClaim};

impl EnvironmentRegistry {
    /// Claim the exclusive right to run this environment's retained inspect
    /// for one dead member. Exactly one death signal per member claims it;
    /// repeated EOF/reset for the same member claims nothing. Unknown
    /// environments claim nothing.
    pub fn begin_inspect(&self, environment_ref: &str, agent_uuid: &str) -> Option<InspectClaim> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.entries.contains_key(environment_ref) {
            return None;
        }
        let key = (environment_ref.to_string(), agent_uuid.to_string());
        if !state.inspect_claims.insert(key) {
            return None;
        }
        Some(InspectClaim {
            environment_ref: environment_ref.to_string(),
        })
    }

    /// Commit a successful inspect outcome onto the authoritative environment
    /// aggregate: the inspect result's metadata object is merged over the
    /// create-time metadata (create keys survive unless the inspect names
    /// them). Callers invoke this BEFORE member removal; the outcome also
    /// survives an environment that is already empty. A removed environment
    /// is a no-op, never a panic.
    pub fn record_inspect_success(&self, claim: InspectClaim, metadata: serde_json::Value) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // A later successful inspect supersedes a previously persisted
        // inspect failure: clear the sticky flag so the environment's most
        // recent inspect outcome is what get_containers reports, and drop
        // the stale inspect error unless a kill failure now owns last_error.
        let had_inspect_failure = state.inspect_failures.remove(&claim.environment_ref);
        if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
            if had_inspect_failure && record.status != EnvironmentStatus::CleanupFailed {
                record.last_error = None;
            }
            match (record.metadata.as_object_mut(), metadata) {
                (Some(existing), serde_json::Value::Object(incoming)) => {
                    for (key, value) in incoming {
                        existing.insert(key, value);
                    }
                }
                (_, incoming) => record.metadata = incoming,
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }

    /// Persist an inspect failure truthfully: the actionable error is
    /// retained on the aggregate, and the retained inspect argv survives so
    /// the inspect can be retried. A removed environment is a no-op.
    pub fn record_inspect_failure(&self, claim: InspectClaim, error: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.entries.contains_key(&claim.environment_ref) {
            state.inspect_failures.insert(claim.environment_ref.clone());
            if let Some(record) = state.entries.get_mut(&claim.environment_ref) {
                record.last_error = Some(error.to_string());
            }
        }
        drop(state);
        self.journal_ref(&claim.environment_ref);
    }
}
