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
    /// Unix timestamp (seconds) of last execution, or 0 if never run.
    pub last_run_at: u64,
}

/// Check whether a cron job is due based on current time.
pub fn is_job_due(job: &CronJob, now_secs: u64) -> bool {
    if !job.enabled {
        return false;
    }
    match &job.schedule {
        CronSchedule::Interval { seconds } => {
            if *seconds == 0 {
                return false;
            }
            job.last_run_at == 0 || now_secs >= job.last_run_at.saturating_add(*seconds)
        }
        CronSchedule::Cron { .. } => {
            // Cron expression evaluation is not yet implemented.
            false
        }
    }
}

/// Returns an unsupported scheduling reason when a job cannot execute yet.
pub fn unsupported_schedule_reason(job: &CronJob) -> Option<&'static str> {
    if !job.enabled {
        return None;
    }
    match job.schedule {
        CronSchedule::Cron { .. } => Some("cron expression execution not implemented"),
        CronSchedule::Interval { .. } => None,
    }
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

    /// Atomically add a job only if no job with the same name exists.
    /// Returns `Ok(true)` if added, `Ok(false)` if a duplicate was found.
    fn add_if_absent(&self, job: CronJob) -> Result<bool, DomainError>;

    /// Remove a job by id.
    fn remove(&self, id: &str) -> Result<(), DomainError>;

    /// Enable or disable a job.
    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError>;

    /// Find a job by name.
    fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError>;

    /// Record an error on a job (sets last_error field).
    fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError>;

    /// Record the last run timestamp (Unix seconds) on a job.
    fn set_last_run_at(&self, id: &str, timestamp: u64) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(schedule: CronSchedule, enabled: bool, last_run_at: u64) -> CronJob {
        CronJob {
            id: "test".to_string(),
            name: "test".to_string(),
            message: "test".to_string(),
            schedule,
            enabled,
            deliver_to: None,
            last_error: None,
            last_run_at,
        }
    }

    #[test]
    fn test_disabled_job_is_never_due() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, false, 0);
        assert!(!is_job_due(&job, 1000));
    }

    #[test]
    fn test_never_run_job_is_due() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, true, 0);
        assert!(is_job_due(&job, 1000));
    }

    #[test]
    fn test_job_not_yet_due() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, true, 1000);
        assert!(!is_job_due(&job, 1030)); // Only 30s elapsed, need 60
    }

    #[test]
    fn test_job_exactly_due() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, true, 1000);
        assert!(is_job_due(&job, 1060));
    }

    #[test]
    fn test_job_overdue() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, true, 1000);
        assert!(is_job_due(&job, 2000));
    }

    #[test]
    fn test_zero_interval_never_due() {
        let job = make_job(CronSchedule::Interval { seconds: 0 }, true, 0);
        assert!(!is_job_due(&job, 1000));
    }

    #[test]
    fn test_cron_expression_not_due_until_supported() {
        let job = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            true,
            0,
        );
        assert!(!is_job_due(&job, 1000));
    }

    #[test]
    fn test_unsupported_schedule_reason_only_for_enabled_cron_expression() {
        let enabled = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            true,
            0,
        );
        assert_eq!(
            unsupported_schedule_reason(&enabled),
            Some("cron expression execution not implemented")
        );

        let disabled = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            false,
            0,
        );
        assert_eq!(unsupported_schedule_reason(&disabled), None);
    }
}
