//! Port traits for coding job coordination.
//!
//! These define what the application layer needs from the outside world.
//! Infrastructure adapters implement these traits.

/// Port for validating repository and ref existence.
pub trait RepoValidator {
    fn repo_exists(&self, repo: &str) -> bool;
    fn ref_exists(&self, repo: &str, base_ref: &str) -> bool;
}

/// Port for resolving skills from the workspace.
pub trait SkillResolver {
    fn skill_exists(&self, name: &str) -> bool;
}

/// Port for checking whether an OS process is still alive.
pub trait ProcessChecker {
    fn is_alive(&self, pid: u32) -> bool;
}

/// A single line result from reading an event log.
#[derive(Debug, Clone)]
pub enum EventLogLine {
    /// Successfully parsed event envelope.
    Valid(super::coding_event::EventEnvelope),
    /// Corrupted or truncated line that could not be parsed.
    Corrupt { line_number: usize, raw: String },
}

/// Port for reading persisted JSONL event logs.
///
/// Each job has its own event log file. The reader discovers job
/// directories and returns their event lines.
pub trait EventLogStore {
    /// Returns the list of job IDs that have event log directories.
    fn discover_jobs(&self) -> Vec<String>;

    /// Reads all lines from the event log for the given job.
    fn read_log(&self, job_id: &str) -> Vec<EventLogLine>;

    /// Appends an event envelope to the log for the given job.
    fn append_event(&mut self, job_id: &str, event: &super::coding_event::EventEnvelope);

    /// Writes or overwrites the jobs index from recovered state.
    fn write_index(&mut self, entries: &[(String, super::coding_job::JobState)]);

    /// Attempts to acquire the coordinator lock. Returns `true` if
    /// acquired, `false` if already held by another process.
    fn try_acquire_lock(&self) -> bool;
}
