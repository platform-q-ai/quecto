//! Adapters of the session-runtime ports the save transaction snapshots
//! (#1860, D5 #1972): the workflow engine's persisted run and the sub-agent
//! registry's historical roster rows. Pure mappings from the runtime
//! objects to domain records; the transaction decides what to do with them.
use std::sync::{Arc, Mutex};

use crate::application::sessions::ports::{HistoricalRosterSource, WorkflowRunSource};
use crate::domain::session::PersistedSubagentRosterEntry;
use crate::domain::workflow::{WorkflowEngine, WorkflowRunPersisted};
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

/// The workflow run of a bound workflow engine.
pub struct WorkflowEngineRunSource(Arc<Mutex<WorkflowEngine>>);

impl WorkflowEngineRunSource {
    pub fn new(engine: Arc<Mutex<WorkflowEngine>>) -> Self {
        Self(engine)
    }
}

impl WorkflowRunSource for WorkflowEngineRunSource {
    fn persisted_run(&self) -> Option<WorkflowRunPersisted> {
        self.0.lock().ok().and_then(|engine| engine.persisted_run())
    }
}

/// The registry's rows as history (#1937): identity, display name,
/// liveness, status and delivery bookkeeping — never a socket path or a
/// pid, which carry no authority after a restart.
pub struct RegistryRosterSource(SubagentRegistry);

impl RegistryRosterSource {
    pub fn new(registry: SubagentRegistry) -> Self {
        Self(registry)
    }
}

impl HistoricalRosterSource for RegistryRosterSource {
    fn roster_rows(&self) -> Vec<PersistedSubagentRosterEntry> {
        let entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .map(|(key, entry)| PersistedSubagentRosterEntry {
                agent_uuid: entry.agent_uuid.as_str().to_string(),
                display_name: entry.effective_display_name(key).to_string(),
                session_key: entry.agent_uuid.as_str().to_string(),
                liveness: entry.persisted_liveness,
                restore_reason: Default::default(),
                parent_id: entry.parent_id.clone(),
                read_only: entry.read_only,
                status: Some(entry.status.to_wire_str().to_string()),
                delivered_message_ordinal: entry.delivered_message_ordinal,
                pending_message_reports: entry.pending_message_reports.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "session_snapshot_sources_tests.rs"]
mod tests;
