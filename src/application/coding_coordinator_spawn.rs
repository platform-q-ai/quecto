// Spawn-management methods for CodingCoordinator.
//
// This is a `#[path]` submodule of coding_coordinator. `super::` refers
// to the coding_coordinator module. Explicit `use` statements list all
// dependencies for contributor clarity.

use crate::application::coding_spawn_manager::{
    SpawnDecision, SpawnError, SpawnManager, SpawnPolicy, SpawnRequest, SpawnResult,
};
use crate::domain::coding_command::CommandError;
use crate::domain::coding_event::EventSource;
use crate::domain::coding_job::JobState;
use crate::domain::coding_ports::{RepoValidator, SkillResolver};

use super::CodingCoordinator;
use super::EventMeta;

impl<R: RepoValidator, S: SkillResolver> CodingCoordinator<R, S> {
    /// Initialize a spawn manager for a job with the given policy.
    pub fn init_spawn_manager(&mut self, job_id: &str, policy: SpawnPolicy) {
        self.spawn_managers
            .insert(job_id.to_string(), SpawnManager::new(policy));
    }

    /// Get a reference to the spawn manager for a job.
    pub fn spawn_manager(&self, job_id: &str) -> Option<&SpawnManager> {
        self.spawn_managers.get(job_id)
    }

    /// Get a mutable reference to the spawn manager for a job.
    pub fn spawn_manager_mut(&mut self, job_id: &str) -> Option<&mut SpawnManager> {
        self.spawn_managers.get_mut(job_id)
    }

    /// Evaluate a spawn request and emit the decision event.
    pub fn evaluate_spawn(
        &mut self,
        job_id: &str,
        req: &SpawnRequest,
    ) -> Result<SpawnDecision, CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        if job.state != JobState::Running && job.state != JobState::Blocked {
            return Err(CommandError::InvalidTransition);
        }
        let rid = job.run_id.clone();

        // Emit spawn.request event from worker
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Worker,
                event_type: "spawn.request".to_string(),
                payload: serde_json::json!({
                    "request_id": req.request_id,
                    "agent_type": req.agent_type,
                    "scope": req.scope,
                    "expected_output": req.expected_output,
                }),
            },
        );

        let mgr = self
            .spawn_managers
            .get_mut(job_id)
            .ok_or(CommandError::NotFound)?;
        let decision = mgr.evaluate(req);

        // Emit spawn.decision event from coordinator
        let mut payload = serde_json::json!({
            "request_id": decision.request_id,
            "agent_type": decision.agent_type,
            "approved": decision.approved,
        });
        if let Some(reason) = &decision.reason {
            payload["reason"] = serde_json::json!(reason);
        }
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "spawn.decision".to_string(),
                payload,
            },
        );

        Ok(decision)
    }

    /// Record a spawn result and emit the result event.
    pub fn record_spawn_result(
        &mut self,
        job_id: &str,
        result: SpawnResult,
    ) -> Result<(), CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        let rid = job.run_id.clone();

        // Build event payload before moving result into the manager.
        let mut payload = serde_json::json!({
            "request_id": result.request_id,
            "state": result.state,
        });
        if let Some(ref summary) = result.summary {
            payload["summary"] = serde_json::json!(summary);
        }
        if !result.artifact_refs.is_empty() {
            payload["artifact_refs"] = serde_json::json!(result.artifact_refs);
        }

        let mgr = self
            .spawn_managers
            .get_mut(job_id)
            .ok_or(CommandError::NotFound)?;
        mgr.record_result(result)
            .map_err(|_| CommandError::NotFound)?;

        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "spawn.result".to_string(),
                payload,
            },
        );

        Ok(())
    }

    /// Try recording a result for an unknown request_id. Returns error
    /// indicating the result should be discarded.
    pub fn try_record_spawn_result(
        &mut self,
        job_id: &str,
        result: SpawnResult,
    ) -> Result<(), SpawnError> {
        let mgr = self
            .spawn_managers
            .get_mut(job_id)
            .ok_or(SpawnError::UnknownRequestId)?;
        mgr.record_result(result)
    }

    /// Cancel all active child spawns for a job and emit spawn.result
    /// events for each canceled spawn.
    pub fn cancel_child_spawns(&mut self, job_id: &str) -> Result<Vec<String>, CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        let rid = job.run_id.clone();
        let mgr = self
            .spawn_managers
            .get_mut(job_id)
            .ok_or(CommandError::NotFound)?;
        let canceled = mgr.cancel_all();
        for request_id in &canceled {
            self.emit(
                &rid,
                job_id,
                EventMeta {
                    source: EventSource::Coordinator,
                    event_type: "spawn.result".to_string(),
                    payload: serde_json::json!({
                        "request_id": request_id,
                        "state": "canceled",
                        "summary": "parent canceled",
                    }),
                },
            );
        }
        Ok(canceled)
    }
}
