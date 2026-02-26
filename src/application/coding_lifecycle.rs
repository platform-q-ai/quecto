//! Coding lifecycle driver — tick-based orchestrator for job progression.
//!
//! The driver advances coding jobs through their states on each `tick()`:
//! 1. Queued → Preparing (begin preparation)
//! 2. Preparing → Running (clone repo, launch worker, mark ready)
//! 3. Running → poll events, detect exits → Succeeded/Failed
//! 4. Canceled (while running) → kill worker

use std::collections::HashMap;

use super::coding_coordinator::{CodingCoordinator, FailureInfo, SuccessInfo};
use crate::domain::coding_command::RunRequest;
use crate::domain::coding_job::{ErrorCode, JobState};
use crate::domain::coding_ports::{
    CloneJobParams, RepoMirrorStore, RepoValidator, SkillResolver, WorkerEvent, WorkerLaunchConfig,
    WorkerRuntime, WorkerStatus,
};

/// Default resource limits for worker launch.
const DEFAULT_MAX_MEMORY_MB: u32 = 512;
const DEFAULT_MAX_CPU_SECONDS: u32 = 120;
const DEFAULT_MAX_WALL_SECONDS: u32 = 300;
const DEFAULT_MAX_PIDS: u32 = 128;

/// Tick-based lifecycle driver that progresses coding jobs through their
/// states. The interface layer calls `tick()` periodically.
pub struct CodingLifecycleDriver<R: RepoValidator, S: SkillResolver> {
    coordinator: CodingCoordinator<R, S>,
    runtime: Box<dyn WorkerRuntime>,
    mirror: Box<dyn RepoMirrorStore>,
    /// Maps job_id → worker PID for jobs the driver has launched.
    running_workers: HashMap<String, u32>,
    /// Tracks which workers have been killed (to avoid double-kill).
    killed_workers: HashMap<String, bool>,
}

impl<R: RepoValidator, S: SkillResolver> std::fmt::Debug for CodingLifecycleDriver<R, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodingLifecycleDriver")
            .field("running_workers", &self.running_workers.len())
            .field("killed_workers", &self.killed_workers.len())
            .finish()
    }
}

impl<R: RepoValidator, S: SkillResolver> CodingLifecycleDriver<R, S> {
    /// Create a new lifecycle driver.
    pub fn new(
        coordinator: CodingCoordinator<R, S>,
        runtime: Box<dyn WorkerRuntime>,
        mirror: Box<dyn RepoMirrorStore>,
    ) -> Self {
        Self {
            coordinator,
            runtime,
            mirror,
            running_workers: HashMap::new(),
            killed_workers: HashMap::new(),
        }
    }

    /// Access the coordinator (for creating jobs and inspecting state).
    pub fn coordinator(&self) -> &CodingCoordinator<R, S> {
        &self.coordinator
    }

    /// Mutable access to the coordinator.
    pub fn coordinator_mut(&mut self) -> &mut CodingCoordinator<R, S> {
        &mut self.coordinator
    }

    /// Mutable access to the boxed runtime (for downcasting in tests).
    pub fn runtime_box_mut(&mut self) -> &mut Box<dyn WorkerRuntime> {
        &mut self.runtime
    }

    /// Create a job via the coordinator and return its job_id.
    pub fn create_job(&mut self, req: RunRequest) -> Result<String, String> {
        let resp = self.coordinator.run(req).map_err(|e| e.to_string())?;
        Ok(resp.job_id)
    }

    /// Cancel a job via the coordinator.
    pub fn cancel_job(&mut self, job_id: &str) -> Result<(), String> {
        self.coordinator.cancel(job_id).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Check if a worker was killed for a given job.
    pub fn was_worker_killed(&self, job_id: &str) -> bool {
        self.killed_workers.get(job_id).copied().unwrap_or(false)
    }

    /// Main tick — advances all jobs one step through the lifecycle.
    ///
    /// Snapshots job states at the start so each job advances at most
    /// one phase per tick: queued → preparing, preparing → running,
    /// running → poll events/exit, canceled → kill worker.
    pub fn tick(&mut self) {
        // Snapshot all phase-specific job IDs at tick start so a job
        // that transitions in one phase isn't re-processed in the next.
        let canceled_ids = self.job_ids_in_state(JobState::Canceled);
        let queued_ids = self.job_ids_in_state(JobState::Queued);
        let preparing_ids = self.job_ids_in_state(JobState::Preparing);
        let running_entries: Vec<(String, u32)> =
            self.running_workers.clone().into_iter().collect();

        // 1. Cancel first — kill workers immediately
        for job_id in canceled_ids {
            if let Some(pid) = self.running_workers.remove(&job_id) {
                let _ = self.runtime.kill(pid);
                self.killed_workers.insert(job_id, true);
            }
        }

        // 2. Advance queued → preparing
        for job_id in queued_ids {
            let _ = self.coordinator.begin_preparation(&job_id);
        }

        // 3. Advance preparing → running (clone + launch)
        for job_id in preparing_ids {
            self.prepare_job(&job_id);
        }

        // 4. Poll running workers for events and exits
        for (job_id, pid) in running_entries {
            self.poll_worker(&job_id, pid);
        }
    }

    /// Clone repo and launch worker for a preparing job.
    fn prepare_job(&mut self, job_id: &str) {
        let job = match self.coordinator.job(job_id) {
            Some(j) => j.clone(),
            None => return,
        };

        // Clone the repo
        let clone_params = CloneJobParams {
            repo: &job.repo,
            job_id: &job.job_id,
            base_ref: &job.base_ref,
            job_branch: &job.branch,
        };
        let clone_result = self.mirror.clone_for_job(&clone_params);

        if !clone_result.ok {
            let detail = clone_result
                .error
                .unwrap_or_else(|| "clone failed".to_string());
            let _ = self.coordinator.mark_failed(FailureInfo {
                job_id,
                error_code: ErrorCode::Internal,
                error_detail: &detail,
                is_retriable: Some(true),
                duration_ms: Some(clone_result.duration_ms),
            });
            return;
        }

        // Launch the worker
        let job_dir = format!("/tmp/jobs/{}/repo", job_id);
        let config = WorkerLaunchConfig {
            job_dir,
            goal: job.goal.clone(),
            max_memory_mb: DEFAULT_MAX_MEMORY_MB,
            max_cpu_seconds: DEFAULT_MAX_CPU_SECONDS,
            max_wall_seconds: DEFAULT_MAX_WALL_SECONDS,
            max_pids: DEFAULT_MAX_PIDS,
            network_allowed_hosts: vec![],
            die_with_parent: true,
        };

        match self.runtime.launch(&config) {
            Ok(pid) => {
                let _ = self
                    .coordinator
                    .mark_ready(job_id, pid, Some(clone_result.duration_ms));
                self.running_workers.insert(job_id.to_string(), pid);
            }
            Err(err) => {
                let detail = format!("launch failed: {err}");
                let _ = self.coordinator.mark_failed(FailureInfo {
                    job_id,
                    error_code: ErrorCode::Internal,
                    error_detail: &detail,
                    is_retriable: Some(true),
                    duration_ms: Some(clone_result.duration_ms),
                });
            }
        }
    }

    /// Poll a running worker for events and check exit status.
    fn poll_worker(&mut self, job_id: &str, pid: u32) {
        // Drain all available events
        while let Some(event) = self.runtime.read_event(pid) {
            if let WorkerEvent::Valid(envelope) = event {
                let _ = self.coordinator.receive_event(envelope);
            }
        }

        // Check worker status
        match self.runtime.status(pid) {
            WorkerStatus::Running => {}
            WorkerStatus::Exited { status } => {
                self.running_workers.remove(job_id);
                if status == 0 {
                    let _ = self.coordinator.mark_succeeded(SuccessInfo {
                        job_id,
                        summary: "completed",
                        artifacts: vec![],
                        duration_ms: None,
                    });
                } else {
                    let detail = format!("worker exited with status {status}");
                    let _ = self.coordinator.mark_failed(FailureInfo {
                        job_id,
                        error_code: ErrorCode::Internal,
                        error_detail: &detail,
                        is_retriable: None,
                        duration_ms: None,
                    });
                }
            }
            WorkerStatus::Killed { reason } => {
                self.running_workers.remove(job_id);
                let detail = format!("worker killed: {reason}");
                let _ = self.coordinator.mark_failed(FailureInfo {
                    job_id,
                    error_code: ErrorCode::Internal,
                    error_detail: &detail,
                    is_retriable: None,
                    duration_ms: None,
                });
            }
        }
    }

    /// Collect job IDs in a given state.
    fn job_ids_in_state(&self, state: JobState) -> Vec<String> {
        let list = self
            .coordinator
            .list(&crate::domain::coding_command::ListRequest {
                state_filter: Some(vec![state]),
            });
        list.jobs.into_iter().map(|j| j.job_id).collect()
    }
}

#[cfg(test)]
#[path = "coding_lifecycle_tests.rs"]
mod tests;
