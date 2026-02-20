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
}

/// How a cron job is scheduled.
#[derive(Debug, Clone)]
pub enum CronSchedule {
    /// Fire every N seconds.
    Interval { seconds: u64 },
    /// Fire on a cron expression (e.g. "0 9 * * *").
    Cron { expression: String },
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
}
