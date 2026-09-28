//! Persisted `tools.policy` entries that matched no registered tool
//! (#2217, #2247 round 2 N3): the one triage every entrypoint and a reload
//! share. Each entry is kept and ignored, as the config documents; only a
//! typo — a restriction that never applies — is worth a warning. The rest
//! go to the debug log with why they are quiet.

use crate::domain::tool_policy_catalogue::{
    UnmatchedPolicyEntry, classify_unmatched_policy_entry, shown_entry_id,
};

/// The unmatched ids, split: `unknown` are typos worth a warning; `quiet`
/// are kept for a tool built elsewhere, retired, or awaiting its
/// registration, each with its class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnmatchedPolicySplit {
    pub unknown: Vec<String>,
    pub quiet: Vec<(String, UnmatchedPolicyEntry)>,
}

/// Split `stable_ids` by class, logging each quiet one at debug.
pub fn split_unmatched_policy_entries(stable_ids: Vec<String>) -> UnmatchedPolicySplit {
    let mut split = UnmatchedPolicySplit::default();
    for stable_id in stable_ids {
        let class = classify_unmatched_policy_entry(&stable_id);
        // Logged bounded and escaped: the key is whatever the file holds.
        let shown = shown_entry_id(&stable_id);
        match &class {
            UnmatchedPolicyEntry::Unknown => {
                split.unknown.push(stable_id);
                continue;
            }
            UnmatchedPolicyEntry::BundledElsewhere => tracing::debug!(
                target: "tool_policy",
                stable_id = %shown,
                "tools.policy entry names a bundled tool this entrypoint does not build; kept for the others"
            ),
            UnmatchedPolicyEntry::Retired { removed_by } => tracing::debug!(
                target: "tool_policy",
                stable_id = %shown,
                removed_by,
                "tools.policy entry names a retired tool; ignored and safe to delete"
            ),
            UnmatchedPolicyEntry::AwaitingRegistration { source } => tracing::debug!(
                target: "tool_policy",
                stable_id = %shown,
                source = source.as_str(),
                "tools.policy entry applies when the extension registers"
            ),
        }
        split.quiet.push((stable_id, class));
    }
    split
}

#[cfg(test)]
#[path = "unmatched_policy_tests.rs"]
mod tests;
