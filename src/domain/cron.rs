use super::error::DomainError;

/// A scheduled job.
#[derive(Debug, Clone)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    /// The message to send to the agent when the job fires.
    pub message: String,
    pub schedule: CronSchedule,
    pub enabled: bool,
    /// Optional channel:chat_id to deliver results to.
    pub deliver_to: Option<String>,
    /// Last error from execution (if any).
    pub last_error: Option<String>,
}

/// How a cron job is scheduled.
#[derive(Debug, Clone)]
pub enum CronSchedule {
    /// Fire every N seconds.
    Interval { seconds: u64 },
    /// Fire on a cron expression (e.g. "0 9 * * *").
    Cron { expression: String },
}

/// Result of executing a single cron job.
#[derive(Debug, Clone)]
pub struct CronJobResult {
    /// The job ID.
    pub job_id: String,
    /// The response from the agent (or error message).
    pub response: String,
    /// Whether the execution succeeded.
    pub ok: bool,
    /// Optional delivery target for the result.
    pub deliver_to: Option<String>,
}

/// Port: persistent storage for cron jobs.
pub trait CronStore: Send + Sync {
    /// List all jobs.
    fn list(&self) -> Result<Vec<CronJob>, DomainError>;

    /// Add a new job.
    fn add(&self, job: CronJob) -> Result<(), DomainError>;

    /// Remove a job by id.
    fn remove(&self, id: &str) -> Result<(), DomainError>;

    /// Enable or disable a job.
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError>;

    /// Find a job by name.
    fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError>;

    /// Record an error on a job (sets last_error field).
    fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError>;
}
