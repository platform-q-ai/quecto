//! Child agent spawn management — policy evaluation, tracking, and result routing.
//!
//! The `SpawnManager` handles spawn.request evaluation, enforces policy limits
//! (allowlist, depth, budget), tracks active child agents, and routes results.

use std::collections::HashMap;

/// Policy configuration for child agent spawning.
#[derive(Debug, Clone, Default)]
pub struct SpawnPolicy {
    pub allow_types: Vec<String>,
    pub max_depth: u32,
    pub max_spawns_per_job: usize,
}

/// A spawn request from a worker.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub request_id: String,
    pub agent_type: String,
    pub scope: String,
    pub expected_output: Option<String>,
}

/// Decision result from evaluating a spawn request.
#[derive(Debug, Clone)]
pub struct SpawnDecision {
    pub request_id: String,
    pub agent_type: String,
    pub approved: bool,
    pub reason: Option<String>,
    pub dedup_of: Option<String>,
}

/// Result from a completed child agent.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub request_id: String,
    pub state: String,
    pub summary: Option<String>,
    pub artifact_refs: Vec<String>,
}

/// Tracks an active child agent.
#[derive(Debug, Clone)]
struct ActiveSpawn {
    request_id: String,
    expected_output: Option<String>,
    terminal: bool,
}

/// Error from spawn result recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnError {
    UnknownRequestId,
    AlreadyTerminal,
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRequestId => f.write_str("unknown request_id"),
            Self::AlreadyTerminal => f.write_str("spawn already terminal"),
        }
    }
}

/// Manages child agent spawning for a single coding job.
pub struct SpawnManager {
    policy: SpawnPolicy,
    current_depth: u32,
    spawns: Vec<ActiveSpawn>,
    active_by_key: HashMap<String, String>,
    results: Vec<SpawnResult>,
}

impl std::fmt::Debug for SpawnManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnManager")
            .field("spawn_count", &self.spawns.len())
            .field("result_count", &self.results.len())
            .finish()
    }
}

impl SpawnManager {
    pub fn new(policy: SpawnPolicy) -> Self {
        Self {
            policy,
            current_depth: 0,
            spawns: Vec::new(),
            active_by_key: HashMap::new(),
            results: Vec::new(),
        }
    }

    pub fn set_current_depth(&mut self, depth: u32) {
        self.current_depth = depth;
    }

    pub fn current_depth(&self) -> u32 {
        self.current_depth
    }

    pub fn spawn_count(&self) -> usize {
        self.spawns.len()
    }

    pub fn results(&self) -> &[SpawnResult] {
        &self.results
    }

    /// Check if a request_id is known (was previously approved).
    pub fn is_known_request(&self, request_id: &str) -> bool {
        self.spawns.iter().any(|s| s.request_id == request_id)
    }

    /// Get the expected_output for an approved spawn.
    pub fn expected_output(&self, request_id: &str) -> Option<&str> {
        self.spawns
            .iter()
            .find(|s| s.request_id == request_id)
            .and_then(|s| s.expected_output.as_deref())
    }

    /// Check if a spawn is in terminal state.
    pub fn is_terminal(&self, request_id: &str) -> bool {
        self.spawns
            .iter()
            .find(|s| s.request_id == request_id)
            .map(|s| s.terminal)
            .unwrap_or(false)
    }

    /// Evaluate a spawn request against policy.
    pub fn evaluate(&mut self, req: &SpawnRequest) -> SpawnDecision {
        // Check depth limit
        if self.current_depth >= self.policy.max_depth {
            return SpawnDecision {
                request_id: req.request_id.clone(),
                agent_type: req.agent_type.clone(),
                approved: false,
                reason: Some("max spawn depth is reached".to_string()),
                dedup_of: None,
            };
        }

        // Check per-job spawn limit
        if self.spawns.len() >= self.policy.max_spawns_per_job {
            return SpawnDecision {
                request_id: req.request_id.clone(),
                agent_type: req.agent_type.clone(),
                approved: false,
                reason: Some("per-job spawn limit is reached".to_string()),
                dedup_of: None,
            };
        }

        // Check allowlist
        if !self.policy.allow_types.iter().any(|t| t == &req.agent_type) {
            return SpawnDecision {
                request_id: req.request_id.clone(),
                agent_type: req.agent_type.clone(),
                approved: false,
                reason: Some("agent type is not allowed".to_string()),
                dedup_of: None,
            };
        }

        // Deduplication check
        let key = format!("{}::{}", req.agent_type, req.scope);
        let dedup_of = self.active_by_key.get(&key).cloned();

        // Approved — track the spawn
        self.spawns.push(ActiveSpawn {
            request_id: req.request_id.clone(),
            expected_output: req.expected_output.clone(),
            terminal: false,
        });
        self.active_by_key
            .entry(key)
            .or_insert_with(|| req.request_id.clone());

        SpawnDecision {
            request_id: req.request_id.clone(),
            agent_type: req.agent_type.clone(),
            approved: true,
            reason: None,
            dedup_of,
        }
    }

    /// Record a spawn result. Returns error if request_id is unknown
    /// or if the spawn is already in a terminal state.
    pub fn record_result(&mut self, result: SpawnResult) -> Result<(), SpawnError> {
        let spawn = self
            .spawns
            .iter_mut()
            .find(|s| s.request_id == result.request_id)
            .ok_or(SpawnError::UnknownRequestId)?;
        if spawn.terminal {
            return Err(SpawnError::AlreadyTerminal);
        }
        spawn.terminal = true;
        self.results.push(result);
        Ok(())
    }

    /// Cancel all active (non-terminal) child agents. Returns the
    /// request_ids of newly canceled spawns.
    pub fn cancel_all(&mut self) -> Vec<String> {
        let mut canceled = Vec::new();
        for spawn in &mut self.spawns {
            if !spawn.terminal {
                spawn.terminal = true;
                canceled.push(spawn.request_id.clone());
            }
        }
        canceled
    }

    /// Clear results. For test isolation.
    #[doc(hidden)]
    pub fn clear_results_for_testing(&mut self) {
        self.results.clear();
    }
}

#[cfg(test)]
#[path = "coding_spawn_manager_tests.rs"]
mod tests;
