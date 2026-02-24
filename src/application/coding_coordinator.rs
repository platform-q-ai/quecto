//! Coding job coordinator — manages lifecycle, events, and command API.
//!
//! The coordinator is the single owner of job state transitions and event
//! emission. All mutations flow through its command methods: `run()`,
//! `status()`, `cancel()`, `cleanup()`, `list()`.

use std::collections::HashMap;

use super::coding_todos::TodoTracker;
use crate::domain::coding_command::{
    CancelResponse, CleanupResponse, CommandError, ListJobEntry, ListRequest, ListResponse,
    RunRequest, RunResponse, StatusResponse,
};
use crate::domain::coding_contract::{SeqScope, next_seq_for, validate_and_track_event};
use crate::domain::coding_event::{
    EventEnvelope, EventSource, is_compatible_version, is_known_event_type,
};
use crate::domain::coding_job::{
    CancelInitiator, CancelReason, CodingJob, CodingJobInit, ErrorCode, JobState,
};
pub use crate::domain::coding_ports::{RepoValidator, SkillResolver};

// ============================================================================
// Coordinator policy
// ============================================================================

/// Policy configuration for the coordinator.
#[derive(Debug, Clone, Default)]
pub struct CoordinatorPolicy {
    pub skill_denylist: Vec<String>,
    pub skill_allowlist: Vec<String>,
}

impl CoordinatorPolicy {
    /// Returns true if the skill is allowed by policy.
    pub fn is_skill_allowed(&self, name: &str) -> bool {
        if self.skill_denylist.iter().any(|s| s == name) {
            return false;
        }
        if !self.skill_allowlist.is_empty() {
            return self.skill_allowlist.iter().any(|s| s == name);
        }
        true
    }
}

// ============================================================================
// Parameter structs (to stay within clippy too-many-arguments threshold)
// ============================================================================

/// Parameters for marking a job as succeeded.
pub struct SuccessInfo<'a> {
    pub job_id: &'a str,
    pub summary: &'a str,
    pub artifacts: Vec<String>,
    pub duration_ms: Option<u64>,
}

/// Parameters for marking a job as failed.
pub struct FailureInfo<'a> {
    pub job_id: &'a str,
    pub error_code: ErrorCode,
    pub error_detail: &'a str,
    pub is_retriable: Option<bool>,
    pub duration_ms: Option<u64>,
}

/// Bundles source + type + payload for the internal `emit` method,
/// keeping the argument count within the project's clippy threshold.
struct EventMeta {
    source: EventSource,
    event_type: String,
    payload: serde_json::Value,
}

/// Maximum number of events held in memory. Oldest events are drained
/// when this limit is reached.
const MAX_EVENTS: usize = 10_000;

// ============================================================================
// Coordinator
// ============================================================================

/// The coding job coordinator — owns all job state and event emission.
pub struct CodingCoordinator<R: RepoValidator, S: SkillResolver> {
    jobs: HashMap<String, CodingJob>,
    jobs_by_run: HashMap<String, String>,
    events: Vec<EventEnvelope>,
    seq_by_scope: HashMap<SeqScope, u64>,
    policy: CoordinatorPolicy,
    repo_validator: R,
    skill_resolver: S,
    id_counter: u64,
    todo_tracker: TodoTracker,
}

impl<R: RepoValidator, S: SkillResolver> std::fmt::Debug for CodingCoordinator<R, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingCoordinator")
            .field("job_count", &self.jobs.len())
            .field("event_count", &self.events.len())
            .finish()
    }
}

impl<R: RepoValidator, S: SkillResolver> CodingCoordinator<R, S> {
    pub fn new(repo_validator: R, skill_resolver: S, policy: CoordinatorPolicy) -> Self {
        Self {
            jobs: HashMap::new(),
            jobs_by_run: HashMap::new(),
            events: Vec::new(),
            seq_by_scope: HashMap::new(),
            policy,
            repo_validator,
            skill_resolver,
            id_counter: 0,
            todo_tracker: TodoTracker::new(),
        }
    }

    pub fn todo_tracker(&self) -> &TodoTracker {
        &self.todo_tracker
    }

    pub fn todo_tracker_mut(&mut self) -> &mut TodoTracker {
        &mut self.todo_tracker
    }

    pub fn events(&self) -> &[EventEnvelope] {
        &self.events
    }

    /// Clear the event log. Exposed only for test isolation; production
    /// callers should use `cleanup()` which drains per-job state.
    #[doc(hidden)]
    pub fn clear_events_for_testing(&mut self) {
        self.events.clear();
    }

    pub fn job(&self, job_id: &str) -> Option<&CodingJob> {
        self.jobs.get(job_id)
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.id_counter += 1;
        format!("{}_{:06}", prefix, self.id_counter)
    }

    /// Emit an event envelope. Caps the in-memory event log at
    /// `MAX_EVENTS` by draining the oldest half when the limit is hit.
    fn emit(&mut self, run_id: &str, job_id: &str, meta: EventMeta) {
        let scope = SeqScope::new(meta.source, run_id.to_string(), job_id.to_string());
        let seq = next_seq_for(&scope, &self.seq_by_scope);
        let envelope = EventEnvelope {
            v: "1.0".to_string(),
            ts: chrono::Utc::now().to_rfc3339(),
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            source: meta.source,
            event_type: meta.event_type,
            seq,
            payload: meta.payload,
        };
        self.seq_by_scope.insert(scope, seq);
        self.events.push(envelope);
        if self.events.len() > MAX_EVENTS {
            let drain_count = MAX_EVENTS / 2;
            self.events.drain(..drain_count);
        }
    }

    // ── run ──────────────────────────────────────────────────────────────

    pub fn run(&mut self, req: RunRequest) -> Result<RunResponse, CommandError> {
        if !self.repo_validator.repo_exists(&req.repo) {
            return Err(CommandError::InvalidRepo);
        }
        if !self.repo_validator.ref_exists(&req.repo, &req.base_ref) {
            return Err(CommandError::InvalidBaseRef);
        }
        for skill in &req.skills {
            if !self.policy.is_skill_allowed(skill) {
                return Err(CommandError::PolicyDenied);
            }
            if !self.skill_resolver.skill_exists(skill) {
                return Err(CommandError::SkillNotFound);
            }
        }

        let job_id = self.next_id("job");
        let run_id = self.next_id("run");
        let branch = format!("quecto/job/{}", job_id);

        let mut job = CodingJob::new(CodingJobInit {
            job_id: job_id.clone(),
            run_id: run_id.clone(),
            goal: req.goal,
            repo: req.repo,
            base_ref: req.base_ref,
            branch,
        });
        job.priority = req.priority;
        job.profile = req.profile;
        job.labels = req.labels;
        job.skills = req.skills;
        job.max_wall_seconds = req.max_wall_seconds;

        let resp = RunResponse {
            run_id: run_id.clone(),
            job_id: job_id.clone(),
            state: job.state,
        };
        self.jobs_by_run.insert(run_id, job_id.clone());
        self.jobs.insert(job_id, job);
        Ok(resp)
    }

    // ── status ──────────────────────────────────────────────────────────

    pub fn status_by_job_id(&self, job_id: &str) -> Result<StatusResponse, CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        Ok(self.build_status(job))
    }

    pub fn status_by_run_id(&self, run_id: &str) -> Result<StatusResponse, CommandError> {
        let job_id = self.jobs_by_run.get(run_id).ok_or(CommandError::NotFound)?;
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        Ok(self.build_status(job))
    }

    fn build_status(&self, job: &CodingJob) -> StatusResponse {
        StatusResponse {
            job_id: job.job_id.clone(),
            run_id: job.run_id.clone(),
            state: job.state,
            summary: job.summary.clone().or_else(|| Some("status".to_string())),
            progress: job.progress,
            todos: self.todo_tracker.todos_for_job(&job.job_id).to_vec(),
            artifacts: job.artifacts.clone(),
            error_code: job.error_code,
            error_detail: job.error_detail.clone(),
            cancel_reason: job.cancel_reason,
        }
    }

    // ── cancel ───────────────────────────────────────────────────────────

    pub fn cancel(&mut self, job_id: &str) -> Result<CancelResponse, CommandError> {
        self.cancel_with_reason(job_id, CancelReason::UserRequest, CancelInitiator::User)
    }

    pub fn cancel_with_reason(
        &mut self,
        job_id: &str,
        reason: CancelReason,
        initiator: CancelInitiator,
    ) -> Result<CancelResponse, CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(CancelResponse {
                job_id: job.job_id.clone(),
                state: job.state,
            });
        }
        job.transition_to(JobState::Canceled)
            .map_err(|_| CommandError::InvalidTransition)?;
        job.cancel_reason = Some(reason);
        job.cancel_initiated_by = Some(initiator);
        let state = job.state;
        let jid = job.job_id.clone();
        let rid = job.run_id.clone();
        self.todo_tracker.cancel_all(&jid);
        self.emit(
            &rid,
            &jid,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.cancel".to_string(),
                payload: serde_json::json!({
                    "reason": reason.to_string(),
                    "initiated_by": initiator.to_string(),
                }),
            },
        );
        Ok(CancelResponse { job_id: jid, state })
    }

    // ── cleanup ──────────────────────────────────────────────────────────

    pub fn cleanup(
        &mut self,
        job_id: &str,
        keep_artifacts: bool,
    ) -> Result<CleanupResponse, CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        if !job.state.is_terminal() {
            return Err(CommandError::JobNotTerminal);
        }
        // MVP: keep_artifacts is accepted but not yet acted upon.
        // A future PR will implement selective artifact preservation.
        let _ = keep_artifacts;
        let run_id = job.run_id.clone();
        self.todo_tracker.remove_job(job_id);
        self.jobs.remove(job_id);
        self.jobs_by_run.remove(&run_id);
        Ok(CleanupResponse {
            job_id: job_id.to_string(),
            cleaned: true,
        })
    }

    // ── list ─────────────────────────────────────────────────────────────

    pub fn list(&self, req: &ListRequest) -> ListResponse {
        let jobs = self
            .jobs
            .values()
            .filter(|j| {
                req.state_filter
                    .as_ref()
                    .map(|f| f.contains(&j.state))
                    .unwrap_or(true)
            })
            .map(|j| ListJobEntry {
                job_id: j.job_id.clone(),
                run_id: j.run_id.clone(),
                state: j.state,
                summary: j.summary.clone(),
            })
            .collect();
        ListResponse { jobs }
    }

    // ── lifecycle transitions ────────────────────────────────────────────

    pub fn begin_preparation(&mut self, job_id: &str) -> Result<(), CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Preparing)
            .map_err(|_| CommandError::InvalidTransition)?;
        let rid = job.run_id.clone();
        let goal = job.goal.clone();
        let base_ref = job.base_ref.clone();
        let branch = job.branch.clone();
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.start".to_string(),
                payload: serde_json::json!({
                    "goal": goal,
                    "base_ref": base_ref,
                    "branch": branch,
                }),
            },
        );
        Ok(())
    }

    pub fn mark_ready(
        &mut self,
        job_id: &str,
        worker_pid: u32,
        clone_duration_ms: Option<u64>,
    ) -> Result<(), CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Running)
            .map_err(|_| CommandError::InvalidTransition)?;
        job.worker_pid = Some(worker_pid);
        let rid = job.run_id.clone();
        let mut payload = serde_json::json!({"worker_pid": worker_pid});
        if let Some(ms) = clone_duration_ms {
            payload["clone_duration_ms"] = serde_json::json!(ms);
        }
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.ready".to_string(),
                payload,
            },
        );
        Ok(())
    }

    pub fn mark_succeeded(&mut self, info: SuccessInfo<'_>) -> Result<(), CommandError> {
        let job = self
            .jobs
            .get_mut(info.job_id)
            .ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Succeeded)
            .map_err(|_| CommandError::InvalidTransition)?;
        job.summary = Some(info.summary.to_string());
        job.artifacts = info.artifacts.clone();
        job.duration_ms = info.duration_ms;
        let rid = job.run_id.clone();
        let mut payload = serde_json::json!({
            "state": "succeeded",
            "summary": info.summary,
            "artifacts": info.artifacts,
        });
        if let Some(ms) = info.duration_ms {
            payload["duration_ms"] = serde_json::json!(ms);
        }
        self.emit(
            &rid,
            info.job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.end".to_string(),
                payload,
            },
        );
        Ok(())
    }

    pub fn mark_failed(&mut self, info: FailureInfo<'_>) -> Result<(), CommandError> {
        let job = self
            .jobs
            .get_mut(info.job_id)
            .ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Failed)
            .map_err(|_| CommandError::InvalidTransition)?;
        job.error_code = Some(info.error_code);
        job.error_detail = Some(info.error_detail.to_string());
        job.is_retriable = info.is_retriable;
        job.duration_ms = info.duration_ms;
        let rid = job.run_id.clone();
        let mut payload = serde_json::json!({
            "state": "failed",
            "summary": info.error_detail,
            "error_code": info.error_code.to_string(),
        });
        if let Some(detail) = &job.error_detail {
            payload["error_detail"] = serde_json::json!(detail);
        }
        if let Some(r) = info.is_retriable {
            payload["is_retriable"] = serde_json::json!(r);
        }
        if let Some(ms) = info.duration_ms {
            payload["duration_ms"] = serde_json::json!(ms);
        }
        self.emit(
            &rid,
            info.job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.end".to_string(),
                payload,
            },
        );
        Ok(())
    }

    pub fn mark_blocked(
        &mut self,
        job_id: &str,
        reason: &str,
        needs: Option<&str>,
    ) -> Result<(), CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Blocked)
            .map_err(|_| CommandError::InvalidTransition)?;
        let rid = job.run_id.clone();
        let mut payload = serde_json::json!({"reason": reason});
        if let Some(n) = needs {
            payload["needs"] = serde_json::json!(n);
        }
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.blocked".to_string(),
                payload,
            },
        );
        Ok(())
    }

    pub fn mark_resumed(&mut self, job_id: &str, reason: &str) -> Result<(), CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        job.transition_to(JobState::Running)
            .map_err(|_| CommandError::InvalidTransition)?;
        let rid = job.run_id.clone();
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Coordinator,
                event_type: "job.resumed".to_string(),
                payload: serde_json::json!({"reason": reason}),
            },
        );
        Ok(())
    }

    pub fn record_worker_progress(
        &mut self,
        job_id: &str,
        progress: u32,
        summary: &str,
    ) -> Result<(), CommandError> {
        let job = self.jobs.get_mut(job_id).ok_or(CommandError::NotFound)?;
        if job.state != JobState::Running && job.state != JobState::Blocked {
            return Err(CommandError::InvalidTransition);
        }
        job.progress = Some(progress);
        job.summary = Some(summary.to_string());
        let rid = job.run_id.clone();
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Worker,
                event_type: "job.status".to_string(),
                payload: serde_json::json!({
                    "state": "running",
                    "summary": summary,
                    "progress": progress,
                }),
            },
        );
        Ok(())
    }

    pub fn receive_event(&mut self, event: EventEnvelope) -> Result<(), String> {
        if !is_compatible_version(&event.v) {
            return Err(format!("incompatible version: {}", event.v));
        }
        // Validate known event types against the full contract. Unknown
        // event types are accepted with a warning (forward-compatible).
        if is_known_event_type(&event.event_type) {
            validate_and_track_event(&event, &mut self.seq_by_scope)
                .map_err(|e| format!("contract violation: {e}"))?;
        }
        self.events.push(event);
        if self.events.len() > MAX_EVENTS {
            let drain_count = MAX_EVENTS / 2;
            self.events.drain(..drain_count);
        }
        Ok(())
    }

    pub fn emit_worker_event(
        &mut self,
        job_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), CommandError> {
        let job = self.jobs.get(job_id).ok_or(CommandError::NotFound)?;
        if job.state != JobState::Running && job.state != JobState::Blocked {
            return Err(CommandError::InvalidTransition);
        }
        let rid = job.run_id.clone();
        self.emit(
            &rid,
            job_id,
            EventMeta {
                source: EventSource::Worker,
                event_type: event_type.to_string(),
                payload,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "coding_coordinator_tests.rs"]
mod tests;
