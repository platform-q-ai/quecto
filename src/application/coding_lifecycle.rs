//! Coding lifecycle driver — tick-based orchestrator for job progression.
//!
//! The driver advances coding jobs through their states on each `tick()`:
//! 1. Queued → Preparing (begin preparation)
//! 2. Preparing → Running (clone repo, launch worker, mark ready)
//! 3. Running → poll events, detect exits → Succeeded/Failed
//! 4. Canceled (while running) → kill worker

use std::collections::HashMap;
use std::time::Instant;

use super::coding_coordinator::{CodingCoordinator, FailureInfo, SuccessInfo};
use crate::domain::coding_command::RunRequest;
use crate::domain::coding_job::{ErrorCode, JobState};
use crate::domain::coding_ports::{
    CloneJobParams, RepoMirrorStore, RepoValidator, SkillResolver, WorkerEvent, WorkerLaunchConfig,
    WorkerRuntime, WorkerStatus,
};

/// Per-worker tracking state. Bundled into a single struct so the PID,
/// launch instant, and killed flag are structurally co-located — making
/// it impossible to have one without the others.
#[derive(Debug)]
struct WorkerState {
    pid: u32,
    /// Wall-clock instant when the worker was launched.
    /// `None` when the job has no `max_wall_seconds` (skip timeout checks).
    started_at: Option<Instant>,
    /// Whether this worker has been killed (to avoid double-kill).
    killed: bool,
}

/// Arguments for `fail_job()` helper (keeps argument count within clippy limit).
struct FailArgs<'a> {
    job_id: &'a str,
    detail: &'a str,
    retriable: Option<bool>,
    duration_ms: Option<u64>,
}

/// Default resource limits for worker launch.
const DEFAULT_MAX_MEMORY_MB: u32 = 512;
const DEFAULT_MAX_CPU_SECONDS: u32 = 120;
const DEFAULT_MAX_WALL_SECONDS: u32 = 300;
const DEFAULT_MAX_PIDS: u32 = 128;

/// Maximum events to drain from a single worker per tick to prevent
/// a misbehaving worker from stalling the entire tick loop.
const MAX_EVENTS_PER_POLL: usize = 1_000;

/// Tick-based lifecycle driver that progresses coding jobs through their
/// states. The interface layer calls `tick()` periodically.
pub struct CodingLifecycleDriver<R: RepoValidator, S: SkillResolver> {
    coordinator: CodingCoordinator<R, S>,
    runtime: Box<dyn WorkerRuntime>,
    mirror: Box<dyn RepoMirrorStore>,
    /// Maps job_id → worker state (pid, started_at, killed flag).
    workers: HashMap<String, WorkerState>,
}

impl<R: RepoValidator, S: SkillResolver> std::fmt::Debug for CodingLifecycleDriver<R, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let running = self.workers.values().filter(|w| !w.killed).count();
        let killed = self.workers.values().filter(|w| w.killed).count();
        f.debug_struct("CodingLifecycleDriver")
            .field("running_workers", &running)
            .field("killed_workers", &killed)
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
            workers: HashMap::new(),
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

    /// Mutable access to the runtime for downcasting in tests.
    ///
    /// Returns `&mut dyn WorkerRuntime` rather than exposing the `Box`
    /// wrapper. Callers use `as_any_mut()` for concrete downcast.
    pub fn runtime_mut(&mut self) -> &mut dyn WorkerRuntime {
        &mut *self.runtime
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
        self.workers.get(job_id).is_some_and(|w| w.killed)
    }

    /// Remove tracking state for a terminated job.
    ///
    /// Call this after the interface layer has fully cleaned up the job
    /// (e.g. removed repo directory). Prevents unbounded growth of
    /// the killed_workers set.
    pub fn forget_job(&mut self, job_id: &str) {
        self.workers.remove(job_id);
    }

    /// Main tick — advances all jobs one step through the lifecycle.
    ///
    /// Snapshots job states at the start so each job advances at most
    /// one phase per tick: queued → preparing, preparing → running,
    /// running → poll events/exit, canceled → kill worker.
    pub fn tick(&mut self) {
        // Short-circuit: skip expensive scans when there's nothing to do.
        if !self.coordinator.has_active_jobs() && self.workers.is_empty() {
            return;
        }

        // Snapshot all phase-specific job IDs at tick start so a job
        // that transitions in one phase isn't re-processed in the next.
        let canceled_ids = self.job_ids_in_state(JobState::Canceled);
        let queued_ids = self.job_ids_in_state(JobState::Queued);
        let preparing_ids = self.job_ids_in_state(JobState::Preparing);
        let running_entries: Vec<(String, u32)> = self
            .workers
            .iter()
            .filter(|(_, w)| !w.killed)
            .map(|(k, w)| (k.clone(), w.pid))
            .collect();

        // 1. Cancel first — kill workers immediately
        for job_id in canceled_ids {
            if let Some(w) = self.workers.get_mut(&job_id) {
                if !w.killed {
                    let pid = w.pid;
                    if let Err(e) = self.runtime.kill(pid) {
                        tracing::warn!(job_id, pid, error = %e, "failed to kill worker");
                    }
                    self.runtime.cleanup(pid);
                    w.killed = true;
                }
            }
        }

        // 2. Advance queued → preparing
        for job_id in queued_ids {
            if let Err(e) = self.coordinator.begin_preparation(&job_id) {
                tracing::warn!(job_id, error = %e, "failed to begin preparation");
            }
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

    /// Ensure a mirror exists for the repo. Returns `false` if creation failed
    /// (the job is marked as failed in that case).
    fn ensure_mirror(&mut self, job_id: &str, repo: &str) -> bool {
        if self.mirror.mirror_exists(repo) {
            return true;
        }
        if let Some(remote_url) = self.mirror.resolve_local_remote(repo) {
            let result = self.mirror.create_mirror(repo, &remote_url);
            if result.ok {
                return true;
            }
            let detail = result
                .error
                .unwrap_or_else(|| "mirror creation failed".to_string());
            self.fail_job(FailArgs {
                job_id,
                detail: &detail,
                retriable: Some(true),
                duration_ms: Some(result.duration_ms),
            });
            return false;
        }
        let detail = format!("no mirror for repo '{repo}' and cannot resolve local path");
        self.fail_job(FailArgs {
            job_id,
            detail: &detail,
            retriable: Some(false),
            duration_ms: None,
        });
        false
    }

    /// Mark a job as failed with the given details.
    fn fail_job(&mut self, args: FailArgs<'_>) {
        if let Err(e) = self.coordinator.mark_failed(FailureInfo {
            job_id: args.job_id,
            error_code: ErrorCode::Internal,
            error_detail: args.detail,
            is_retriable: args.retriable,
            duration_ms: args.duration_ms,
        }) {
            tracing::warn!(args.job_id, error = %e, "failed to mark job as failed");
        }
    }

    /// Clone repo and launch worker for a preparing job.
    fn prepare_job(&mut self, job_id: &str) {
        let job = match self.coordinator.job(job_id) {
            Some(j) => j,
            None => return,
        };

        // Extract only the fields we need to avoid cloning the full job.
        let repo = job.repo.clone();
        let jid = job.job_id.clone();
        let rid = job.run_id.clone();
        let base_ref = job.base_ref.clone();
        let branch = job.branch.clone();
        let goal = job.goal.clone();
        let wall_seconds = job
            .max_wall_seconds
            .map(|s| s as u32)
            .unwrap_or(DEFAULT_MAX_WALL_SECONDS);

        // Ensure a mirror exists before cloning.
        if !self.ensure_mirror(job_id, &repo) {
            return;
        }

        // Clone the repo
        let clone_params = CloneJobParams {
            repo: &repo,
            job_id: &jid,
            base_ref: &base_ref,
            job_branch: &branch,
        };
        let clone_result = self.mirror.clone_for_job(&clone_params);

        if !clone_result.ok {
            let detail = clone_result
                .error
                .unwrap_or_else(|| "clone failed".to_string());
            self.fail_job(FailArgs {
                job_id,
                detail: &detail,
                retriable: Some(true),
                duration_ms: Some(clone_result.duration_ms),
            });
            return;
        }

        // Get the job directory from the mirror store (not hardcoded).
        let job_dir = self.mirror.job_repo_path(job_id);
        let config = WorkerLaunchConfig {
            run_id: rid,
            job_id: jid,
            job_dir,
            goal,
            max_memory_mb: DEFAULT_MAX_MEMORY_MB,
            max_cpu_seconds: DEFAULT_MAX_CPU_SECONDS,
            max_wall_seconds: wall_seconds,
            max_pids: DEFAULT_MAX_PIDS,
            network_allowed_hosts: vec![],
            die_with_parent: true,
        };

        // Check whether the job has a wall timeout (used to decide whether
        // to track started_at for per-tick timeout checks).
        let has_wall_timeout = self
            .coordinator
            .job(job_id)
            .and_then(|j| j.max_wall_seconds)
            .is_some_and(|s| s > 0);

        match self.runtime.launch(&config) {
            Ok(pid) => {
                if let Err(e) =
                    self.coordinator
                        .mark_ready(job_id, pid, Some(clone_result.duration_ms))
                {
                    tracing::warn!(job_id, error = %e, "failed to mark job as ready");
                }
                self.workers.insert(
                    job_id.to_string(),
                    WorkerState {
                        pid,
                        started_at: if has_wall_timeout {
                            Some(Instant::now())
                        } else {
                            None
                        },
                        killed: false,
                    },
                );
            }
            Err(err) => {
                let detail = format!("launch failed: {err}");
                self.fail_job(FailArgs {
                    job_id,
                    detail: &detail,
                    retriable: Some(true),
                    duration_ms: Some(clone_result.duration_ms),
                });
            }
        }
    }

    /// Poll a running worker for events and check exit status.
    fn poll_worker(&mut self, job_id: &str, pid: u32) {
        // Skip workers already killed by the cancel phase earlier in
        // this tick (the running_entries snapshot was taken before
        // cancel processing, so it may include stale entries).
        if self.workers.get(job_id).is_some_and(|w| w.killed) {
            return;
        }
        // Check wall-clock timeout before draining events.
        if self.is_wall_timeout_exceeded(job_id) {
            self.handle_wall_timeout(job_id, pid);
            return;
        }

        self.drain_worker_events(job_id, pid);

        match self.runtime.status(pid) {
            WorkerStatus::Running => {}
            WorkerStatus::Exited { status } => {
                self.handle_worker_exit(job_id, pid, status);
            }
            WorkerStatus::Killed { reason } => {
                self.handle_worker_killed(job_id, pid, &reason);
            }
        }
    }

    /// Returns true if the job's `max_wall_seconds` has elapsed since launch.
    ///
    /// Jobs without `max_wall_seconds` have `started_at: None` in their
    /// `WorkerState`, so the check short-circuits immediately (zero cost).
    fn is_wall_timeout_exceeded(&self, job_id: &str) -> bool {
        let started = match self.workers.get(job_id).and_then(|w| w.started_at) {
            Some(s) => s,
            None => return false,
        };
        let max_wall = match self.coordinator.job(job_id) {
            Some(job) => match job.max_wall_seconds {
                Some(s) if s > 0 => s,
                _ => return false,
            },
            None => return false,
        };
        started.elapsed().as_secs() >= max_wall
    }

    /// Kill the worker and mark the job canceled with `WallTimeout` reason.
    ///
    /// NOTE: events emitted by the worker between the last tick and the
    /// timeout are intentionally *not* drained before the kill. This is a
    /// deliberate trade-off: draining first would allow a misbehaving
    /// worker to delay the kill via an event flood (DoS). Diagnostic
    /// data loss is acceptable — the coordinator records `WallTimeout`
    /// as the cancel reason, which is sufficient for debugging.
    fn handle_wall_timeout(&mut self, job_id: &str, pid: u32) {
        tracing::warn!(job_id, pid, "wall timeout exceeded — killing worker");
        // Best-effort drain: read what's immediately available (up to a
        // small cap) without blocking, so we don't lose the last batch
        // of events in the common case while still bounding the work.
        self.drain_worker_events(job_id, pid);
        if let Err(e) = self.runtime.kill(pid) {
            tracing::warn!(job_id, pid, error = %e, "kill failed during wall timeout");
        }
        self.runtime.cleanup(pid);
        if let Some(w) = self.workers.get_mut(job_id) {
            w.killed = true;
        }

        use crate::domain::coding_job::{CancelInitiator, CancelReason};
        if let Err(e) = self.coordinator.cancel_with_reason(
            job_id,
            CancelReason::WallTimeout,
            CancelInitiator::System,
        ) {
            tracing::warn!(job_id, error = %e, "failed to cancel job after wall timeout");
        }
    }

    /// Drain available events from a worker, capped to prevent stalls.
    fn drain_worker_events(&mut self, job_id: &str, pid: u32) {
        let mut drained = 0;
        while drained < MAX_EVENTS_PER_POLL {
            match self.runtime.read_event(pid) {
                Some(WorkerEvent::Valid(envelope)) => {
                    if let Err(e) = self.coordinator.receive_event(envelope) {
                        tracing::warn!(job_id, error = %e, "failed to receive worker event");
                    }
                }
                Some(WorkerEvent::Malformed { raw }) => {
                    tracing::warn!(job_id, pid, raw, "malformed worker event");
                }
                None => break,
            }
            drained += 1;
        }
    }

    /// Handle a worker that exited with a status code.
    fn handle_worker_exit(&mut self, job_id: &str, pid: u32, status: i32) {
        self.workers.remove(job_id);
        self.runtime.cleanup(pid);
        if status == 0 {
            if let Err(e) = self.coordinator.mark_succeeded(SuccessInfo {
                job_id,
                summary: "completed",
                artifacts: vec![],
                duration_ms: None,
            }) {
                tracing::warn!(job_id, error = %e, "failed to mark job as succeeded");
            }
        } else {
            let detail = format!("worker exited with status {status}");
            if let Err(e) = self.coordinator.mark_failed(FailureInfo {
                job_id,
                error_code: ErrorCode::Internal,
                error_detail: &detail,
                is_retriable: None,
                duration_ms: None,
            }) {
                tracing::warn!(job_id, error = %e, "failed to mark job as failed");
            }
        }
    }

    /// Handle a worker that was killed by signal or timeout.
    fn handle_worker_killed(&mut self, job_id: &str, pid: u32, reason: &str) {
        self.workers.remove(job_id);
        self.runtime.cleanup(pid);
        let detail = format!("worker killed: {reason}");
        if let Err(e) = self.coordinator.mark_failed(FailureInfo {
            job_id,
            error_code: ErrorCode::Internal,
            error_detail: &detail,
            is_retriable: None,
            duration_ms: None,
        }) {
            tracing::warn!(job_id, error = %e, "failed to mark killed job as failed");
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
