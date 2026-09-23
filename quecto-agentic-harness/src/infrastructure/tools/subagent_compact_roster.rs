//! The compact sub-agent roster (#524): the registry's live rows, or the rows
//! changed since a notification cursor, in the shape both the `agent_cmd`
//! tool and the UDS roster queries return. It reads only the registry, so it
//! lives beside it; the interface presents it (#1637 moved it out of the
//! interface protocol module, which infrastructure must not import).
use super::subagent_environment_wire::environment_wire;
use super::subagent_registry::{SubagentRegistry, SubagentStatus, effective_status};
use crate::domain::session::SubagentLiveness;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactSubagentRow {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_uuid: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_ref: Option<String>,
}

/// `Default` is the empty roster at sequence 0 (the snapshot's fallback).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactSubagentRoster {
    pub subagents: Vec<CompactSubagentRow>,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unchanged: Option<bool>,
}

pub fn build_compact_subagent_roster(
    registry: &Option<SubagentRegistry>,
    since: Option<u64>,
) -> Result<CompactSubagentRoster, String> {
    let Some(reg) = registry else {
        if since.unwrap_or(0) > 0 {
            return Err("future since cursor".to_string());
        }
        return Ok(CompactSubagentRoster {
            subagents: Vec::new(),
            sequence: 0,
            unchanged: since.map(|_| true),
        });
    };
    let mut rows = Vec::new();
    let current = {
        let guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        let current = guard
            .values()
            .map(|e| e.notification_sequence)
            .max()
            .map(|max| max.max(1))
            .unwrap_or(0);
        if let Some(s) = since {
            if s > current {
                return Err("future since cursor".to_string());
            }
            if s == current {
                return Ok(CompactSubagentRoster {
                    subagents: Vec::new(),
                    sequence: current,
                    unchanged: Some(true),
                });
            }
        }
        for (id, entry) in guard.iter() {
            if since.is_some_and(|s| entry.notification_sequence <= s) {
                continue;
            }
            let effective = effective_status(&guard, id).unwrap_or_else(|| entry.status.clone());
            let terminal = entry.persisted_liveness != SubagentLiveness::Live
                || effective == SubagentStatus::Exited;
            if terminal && (since.is_none() || entry.parent_id.is_none()) {
                continue;
            }
            let display_name = entry.effective_display_name(id).to_string();
            let status = if terminal {
                "dead"
            } else if entry.run_error.is_some() || effective == SubagentStatus::Error {
                "errored"
            } else if effective == SubagentStatus::Idle {
                "idle"
            } else {
                "running"
            }
            .to_string();
            let environment_ref = entry
                .environment_ref
                .clone()
                .or_else(|| environment_wire(entry).map(|environment| environment.environment_ref));
            rows.push(CompactSubagentRow {
                agent_id: display_name,
                agent_uuid: Some(entry.agent_uuid.to_string()),
                status,
                environment_ref,
            });
        }
        current
    };
    rows.sort_by(|a, b| {
        a.agent_id
            .cmp(&b.agent_id)
            .then(a.environment_ref.cmp(&b.environment_ref))
    });
    Ok(CompactSubagentRoster {
        subagents: rows,
        sequence: current,
        unchanged: since.map(|_| false),
    })
}

#[cfg(test)]
#[path = "subagent_compact_roster_tests.rs"]
mod tests;
