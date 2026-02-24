//! Crash recovery for the coding runtime coordinator.
//!
//! On startup the coordinator replays append-only JSONL event logs to
//! reconstruct in-memory state. Orphaned workers (dead PIDs) are
//! detected and their jobs failed. The jobs index is rebuilt from the
//! replayed state — the log is always the source of truth.

use crate::domain::coding_event::{EventEnvelope, EventSource};
use crate::domain::coding_job::JobState;
use crate::domain::coding_ports::{EventLogLine, EventLogStore, ProcessChecker};

use std::collections::HashMap;

// ============================================================================
// Error type
// ============================================================================

/// Errors that can occur during crash recovery.
#[derive(Debug)]
pub enum RecoveryError {
    /// Another coordinator instance already holds the lock.
    LockHeld,
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockHeld => f.write_str("coordinator lock is already held"),
        }
    }
}

// ============================================================================
// Recovery result types
// ============================================================================

/// Per-job recovery outcome.
#[derive(Debug, Clone)]
pub struct RecoveredJob {
    pub job_id: String,
    pub run_id: String,
    pub state: JobState,
    pub worker_pid: Option<u32>,
    pub error_code: Option<String>,
    pub worker_check_performed: bool,
    pub has_todo_events: bool,
}

/// Result of a spawn recovery.
#[derive(Debug, Clone)]
pub struct RecoveredSpawn {
    pub job_id: String,
    pub marked_failed: bool,
}

/// Aggregate recovery result.
#[derive(Debug)]
pub struct RecoveryResult {
    pub jobs: HashMap<String, RecoveredJob>,
    pub spawns: Vec<RecoveredSpawn>,
    pub warnings: Vec<RecoveryWarning>,
    pub events_appended: usize,
    pub index_rewritten: bool,
    /// Operation order for durability verification.
    pub operation_order: Vec<RecoveryOp>,
}

/// Warning emitted during recovery.
#[derive(Debug, Clone)]
pub enum RecoveryWarning {
    EmptyEventLog { job_id: String },
    CorruptedLine { job_id: String, line_number: usize },
}

/// Discrete operations tracked for durability ordering checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOp {
    Append,
    Flush,
    StateUpdate,
}

// ============================================================================
// Internal replay state
// ============================================================================

/// Intermediate state extracted by replaying a single job's event log.
struct ReplayedJob {
    state: JobState,
    terminal: bool,
    worker_pid: Option<u32>,
    has_pending_spawn: bool,
    error_code: Option<String>,
    has_todo_events: bool,
}

/// Replay events for a single job into an intermediate state.
fn replay_events(events: &[EventEnvelope]) -> ReplayedJob {
    let mut state = JobState::Queued;
    let mut terminal = false;
    let mut worker_pid: Option<u32> = None;
    let mut has_pending_spawn = false;
    let mut error_code: Option<String> = None;
    let mut has_todo_events = false;

    for ev in events {
        match ev.event_type.as_str() {
            "job.start" => state = JobState::Preparing,
            "job.ready" => {
                state = JobState::Running;
                if let Some(pid) = ev.payload.get("worker_pid") {
                    if let Some(p) = pid.as_u64() {
                        worker_pid = Some(p as u32);
                    }
                }
            }
            "job.status" => {
                if let Some(s) = ev.payload.get("state").and_then(|v| v.as_str()) {
                    if let Ok(parsed) = s.parse::<JobState>() {
                        state = parsed;
                    }
                }
            }
            "job.blocked" => state = JobState::Blocked,
            "job.resumed" => state = JobState::Running,
            "job.cancel" => {
                state = JobState::Canceled;
                terminal = true;
            }
            "job.end" => {
                if let Some(s) = ev.payload.get("state").and_then(|v| v.as_str()) {
                    if let Ok(parsed) = s.parse::<JobState>() {
                        state = parsed;
                    }
                }
                error_code = ev
                    .payload
                    .get("error_code")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                terminal = true;
            }
            "spawn.request" | "spawn.decision" => has_pending_spawn = true,
            "spawn.result" => has_pending_spawn = false,
            "todo.create" | "todo.update" | "todo.blocked" | "todo.complete" => {
                has_todo_events = true;
            }
            _ => {}
        }
    }

    ReplayedJob {
        state,
        terminal,
        worker_pid,
        has_pending_spawn,
        error_code,
        has_todo_events,
    }
}

// ============================================================================
// Per-job recovery logic (extracted for complexity / line limits)
// ============================================================================

/// Intermediate per-job recovery outcome before deferred event writes.
struct JobRecoveryOutcome {
    recovered: RecoveredJob,
    deferred_job_end: Option<(String, String)>,
    spawn: Option<RecoveredSpawn>,
    deferred_spawn_end: bool,
}

/// Recover a single job from its parsed event lines.
fn recover_single_job<P: ProcessChecker>(
    job_id: &str,
    valid_events: &[EventEnvelope],
    process_checker: &P,
) -> JobRecoveryOutcome {
    let replayed = replay_events(valid_events);
    let mut final_state = replayed.state;
    let mut worker_check_performed = false;
    let mut job_error_code = replayed.error_code;
    let mut deferred_job_end = None;

    if !replayed.terminal {
        match replayed.state {
            JobState::Preparing => {
                final_state = JobState::Failed;
                job_error_code = Some("coordinator_crash".to_string());
                deferred_job_end = Some((job_id.to_string(), get_run_id(valid_events, job_id)));
            }
            JobState::Running | JobState::Blocked => {
                if let Some(pid) = replayed.worker_pid {
                    worker_check_performed = true;
                    if !process_checker.is_alive(pid) {
                        final_state = JobState::Failed;
                        job_error_code = Some("coordinator_crash".to_string());
                        deferred_job_end =
                            Some((job_id.to_string(), get_run_id(valid_events, job_id)));
                    }
                }
            }
            _ => {}
        }
    }

    let mut spawn = None;
    let mut deferred_spawn_end = false;
    if replayed.has_pending_spawn {
        let child_alive = replayed
            .worker_pid
            .map(|pid| process_checker.is_alive(pid))
            .unwrap_or(false);
        if !child_alive {
            deferred_spawn_end = true;
            spawn = Some(RecoveredSpawn {
                job_id: job_id.to_string(),
                marked_failed: true,
            });
        }
    }

    let run_id = get_run_id(valid_events, job_id);
    JobRecoveryOutcome {
        recovered: RecoveredJob {
            job_id: job_id.to_string(),
            run_id,
            state: final_state,
            worker_pid: replayed.worker_pid,
            error_code: job_error_code,
            worker_check_performed,
            has_todo_events: replayed.has_todo_events,
        },
        deferred_job_end,
        spawn,
        deferred_spawn_end,
    }
}

/// Apply deferred event writes and rebuild the index.
fn apply_deferred_writes<E: EventLogStore>(
    event_log_store: &mut E,
    deferred_job_ends: &[(String, String)],
    deferred_spawn_ends: &[String],
    jobs: &HashMap<String, RecoveredJob>,
) -> (usize, Vec<RecoveryOp>) {
    let mut events_appended: usize = 0;
    let mut operation_order = Vec::new();

    for (job_id, run_id) in deferred_job_ends {
        let envelope = make_crash_event(job_id, run_id);
        event_log_store.append_event(job_id, &envelope);
        events_appended += 1;
        operation_order.push(RecoveryOp::Append);
        operation_order.push(RecoveryOp::Flush);
    }

    for job_id in deferred_spawn_ends {
        let run_id = jobs
            .get(job_id)
            .map(|j| j.run_id.clone())
            .unwrap_or_default();
        let envelope = make_spawn_fail_event(job_id, &run_id);
        event_log_store.append_event(job_id, &envelope);
        events_appended += 1;
        operation_order.push(RecoveryOp::Append);
        operation_order.push(RecoveryOp::Flush);
    }

    let index_entries: Vec<(String, JobState)> =
        jobs.values().map(|j| (j.job_id.clone(), j.state)).collect();
    event_log_store.write_index(&index_entries);
    operation_order.push(RecoveryOp::StateUpdate);

    (events_appended, operation_order)
}

// ============================================================================
// Public API
// ============================================================================

/// Run crash recovery using the provided ports.
///
/// 1. Acquires the coordinator lock (fails if held).
/// 2. Discovers all job directories.
/// 3. Replays each job's event log.
/// 4. Detects orphaned workers and marks their jobs as failed.
/// 5. Rebuilds the jobs index from replayed state.
pub fn recover<P: ProcessChecker, E: EventLogStore>(
    process_checker: &P,
    event_log_store: &mut E,
) -> Result<RecoveryResult, RecoveryError> {
    if !event_log_store.try_acquire_lock() {
        return Err(RecoveryError::LockHeld);
    }

    let job_ids = event_log_store.discover_jobs();
    let mut jobs = HashMap::new();
    let mut spawns = Vec::new();
    let mut warnings: Vec<RecoveryWarning> = Vec::new();
    let mut deferred_job_ends: Vec<(String, String)> = Vec::new();
    let mut deferred_spawn_ends: Vec<String> = Vec::new();

    for job_id in &job_ids {
        let lines = event_log_store.read_log(job_id);
        let valid_events = extract_valid_events(job_id, lines, &mut warnings);

        if valid_events.is_empty() {
            warnings.push(RecoveryWarning::EmptyEventLog {
                job_id: job_id.clone(),
            });
            jobs.insert(job_id.clone(), empty_failed_job(job_id));
            continue;
        }

        let outcome = recover_single_job(job_id, &valid_events, process_checker);
        if let Some(je) = outcome.deferred_job_end {
            deferred_job_ends.push(je);
        }
        if outcome.deferred_spawn_end {
            deferred_spawn_ends.push(job_id.clone());
        }
        if let Some(s) = outcome.spawn {
            spawns.push(s);
        }
        jobs.insert(job_id.clone(), outcome.recovered);
    }

    let (events_appended, operation_order) = apply_deferred_writes(
        event_log_store,
        &deferred_job_ends,
        &deferred_spawn_ends,
        &jobs,
    );

    Ok(RecoveryResult {
        jobs,
        spawns,
        warnings,
        events_appended,
        index_rewritten: !job_ids.is_empty(),
        operation_order,
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Extract valid events from log lines, collecting warnings for corrupt ones.
fn extract_valid_events(
    job_id: &str,
    lines: Vec<EventLogLine>,
    warnings: &mut Vec<RecoveryWarning>,
) -> Vec<EventEnvelope> {
    let mut valid = Vec::with_capacity(lines.len());
    for line in lines {
        match line {
            EventLogLine::Valid(env) => valid.push(env),
            EventLogLine::Corrupt { line_number, .. } => {
                warnings.push(RecoveryWarning::CorruptedLine {
                    job_id: job_id.to_string(),
                    line_number,
                });
            }
        }
    }
    valid
}

/// Create a failed-job placeholder for empty event logs.
fn empty_failed_job(job_id: &str) -> RecoveredJob {
    RecoveredJob {
        job_id: job_id.to_string(),
        run_id: job_id.to_string(),
        state: JobState::Failed,
        worker_pid: None,
        error_code: None,
        worker_check_performed: false,
        has_todo_events: false,
    }
}

/// Extract the run_id from the first event or use the job_id as fallback.
fn get_run_id(events: &[EventEnvelope], job_id: &str) -> String {
    events
        .first()
        .map(|e| e.run_id.clone())
        .unwrap_or_else(|| job_id.to_string())
}

/// Build a `job.end` event with `error_code: coordinator_crash`.
fn make_crash_event(job_id: &str, run_id: &str) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        source: EventSource::Coordinator,
        event_type: "job.end".to_string(),
        seq: 0, // Recovery events get seq=0; real tracking happens later
        payload: serde_json::json!({
            "state": "failed",
            "summary": "coordinator crash recovery",
            "error_code": "coordinator_crash",
        }),
    }
}

/// Build a `spawn.result` event with state `failed`.
fn make_spawn_fail_event(job_id: &str, run_id: &str) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        source: EventSource::Coordinator,
        event_type: "spawn.result".to_string(),
        seq: 0,
        payload: serde_json::json!({
            "request_id": "recovery",
            "state": "failed",
        }),
    }
}

#[cfg(test)]
#[path = "coding_crash_recovery_tests.rs"]
mod tests;
