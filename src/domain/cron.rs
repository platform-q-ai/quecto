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
    /// Unix timestamp (seconds) when the job was created. 0 for legacy jobs.
    pub created_at: u64,
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
            job.last_run_at == 0 || now_secs >= job.last_run_at + seconds
        }
        CronSchedule::Cron { expression } => {
            cron_expression_matches(expression, job.last_run_at, now_secs)
        }
    }
}

/// Returns an unsupported scheduling reason when a job cannot execute yet.
/// Now that cron expressions are implemented, this always returns `None`.
pub fn unsupported_schedule_reason(job: &CronJob) -> Option<&'static str> {
    if !job.enabled {
        return None;
    }
    match job.schedule {
        CronSchedule::Cron { .. } | CronSchedule::Interval { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Cron expression evaluator (5-field: minute hour dom month dow)
// ---------------------------------------------------------------------------

/// Check if the current time (as Unix seconds) matches a 5-field cron expression
/// and the job hasn't already run during this matching minute.
fn cron_expression_matches(expression: &str, last_run_at: u64, now_secs: u64) -> bool {
    let fields = match parse_cron_fields(expression) {
        Some(f) => f,
        None => return false, // invalid expression
    };

    let t = unix_to_broken(now_secs);

    // Check each field against the broken-down time.
    if !field_matches(&fields.minute, t.minute) {
        return false;
    }
    if !field_matches(&fields.hour, t.hour) {
        return false;
    }
    if !field_matches(&fields.dom, t.dom) {
        return false;
    }
    if !field_matches(&fields.month, t.month) {
        return false;
    }
    if !field_matches(&fields.dow, t.dow) {
        return false;
    }

    // Don't fire again if already ran during this same minute.
    if last_run_at > 0 {
        let minute_start = now_secs - (now_secs % 60);
        if last_run_at >= minute_start {
            return false;
        }
    }

    true
}

/// Parsed 5-field cron expression.
struct CronFields {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

/// A single cron field can be a wildcard or a set of allowed values.
enum CronField {
    Any,
    Values(Vec<u32>),
}

fn field_matches(field: &CronField, value: u32) -> bool {
    match field {
        CronField::Any => true,
        CronField::Values(vals) => vals.contains(&value),
    }
}

/// Parse a 5-field cron expression string. Returns None for invalid expressions.
fn parse_cron_fields(expression: &str) -> Option<CronFields> {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }

    Some(CronFields {
        minute: parse_single_field(parts[0], 0, 59)?,
        hour: parse_single_field(parts[1], 0, 23)?,
        dom: parse_single_field(parts[2], 1, 31)?,
        month: parse_single_field(parts[3], 1, 12)?,
        dow: parse_single_field(parts[4], 0, 6)?,
    })
}

/// Parse a single cron field (e.g. "*", "5", "1,15", "*/10", "1-5").
fn parse_single_field(field: &str, min: u32, max: u32) -> Option<CronField> {
    if field == "*" {
        return Some(CronField::Any);
    }

    // Handle */step
    if let Some(step_str) = field.strip_prefix("*/") {
        let step: u32 = step_str.parse().ok()?;
        if step == 0 || step > max {
            return None;
        }
        let vals: Vec<u32> = (min..=max).filter(|v| (v - min) % step == 0).collect();
        return Some(CronField::Values(vals));
    }

    // Handle comma-separated list (may include ranges)
    let mut values = Vec::new();
    for part in field.split(',') {
        if let Some((start_str, end_str)) = part.split_once('-') {
            let start: u32 = start_str.parse().ok()?;
            let end: u32 = end_str.parse().ok()?;
            if start > end || start < min || end > max {
                return None;
            }
            values.extend(start..=end);
        } else {
            let val: u32 = part.parse().ok()?;
            if val < min || val > max {
                return None;
            }
            values.push(val);
        }
    }

    if values.is_empty() {
        return None;
    }
    Some(CronField::Values(values))
}

// ---------------------------------------------------------------------------
// Unix timestamp → broken-down time (UTC)
// ---------------------------------------------------------------------------

struct BrokenTime {
    minute: u32,
    hour: u32,
    dom: u32,   // 1-31
    month: u32, // 1-12
    dow: u32,   // 0=Sunday, 6=Saturday
}

/// Convert Unix timestamp (seconds since epoch) to broken-down UTC time.
/// Uses civil date calculation (no external crate needed).
fn unix_to_broken(secs: u64) -> BrokenTime {
    let total_secs = secs as i64;
    let day_secs = total_secs.rem_euclid(86400);
    let hour = (day_secs / 3600) as u32;
    let minute = ((day_secs % 3600) / 60) as u32;

    // Days since epoch (1970-01-01 is a Thursday = dow 4)
    let total_days = total_secs.div_euclid(86400);
    let dow = ((total_days + 4).rem_euclid(7)) as u32; // 0=Sunday

    // Civil date from days since epoch (algorithm from Howard Hinnant)
    let z = total_days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dom = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };

    BrokenTime {
        minute,
        hour,
        dom,
        month,
        dow,
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
            created_at: 1_700_000_000,
        }
    }

    // -----------------------------------------------------------------------
    // Fix 2: created_at field exists on CronJob
    // -----------------------------------------------------------------------

    #[test]
    fn test_cron_job_has_created_at_field() {
        let job = make_job(CronSchedule::Interval { seconds: 60 }, true, 0);
        assert!(job.created_at > 0, "created_at should be set");
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

    // -----------------------------------------------------------------------
    // Fix 1: Cron expression evaluation — these tests assert the CORRECT
    // behavior (expressions ARE evaluated), so they FAIL against the old
    // code that always returns false.
    // -----------------------------------------------------------------------

    #[test]
    fn test_cron_expression_due_when_matching_current_minute() {
        // "* * * * *" matches every minute, so it's always due
        // when the job has never run (last_run_at == 0).
        let job = make_job(
            CronSchedule::Cron {
                expression: "* * * * *".to_string(),
            },
            true,
            0,
        );
        assert!(
            is_job_due(&job, 1_700_000_000),
            "every-minute cron expression should fire when never run"
        );
    }

    #[test]
    fn test_cron_expression_not_due_when_already_ran_this_minute() {
        // "* * * * *" fires every minute. If last_run_at is within the
        // same minute as now, it should NOT be due again.
        let now = 1_700_000_060u64; // some timestamp
        let last_run = now - 10; // ran 10 seconds ago, same minute window
        let job = make_job(
            CronSchedule::Cron {
                expression: "* * * * *".to_string(),
            },
            true,
            last_run,
        );
        assert!(
            !is_job_due(&job, now),
            "should not fire again within the same minute"
        );
    }

    #[test]
    fn test_cron_expression_due_in_next_matching_minute() {
        // "0 9 * * *" = daily at 09:00 UTC. Test with a timestamp at 09:00.
        // 2023-11-14 09:00:00 UTC = 1699952400
        let nine_am = 1_699_952_400u64;
        let job = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            true,
            0, // never run
        );
        assert!(
            is_job_due(&job, nine_am),
            "daily-at-9am job should be due at 09:00 UTC when never run"
        );
    }

    #[test]
    fn test_cron_expression_not_due_at_wrong_hour() {
        // "0 9 * * *" = daily at 09:00 UTC. At 15:00 it's not the matching
        // minute, but the job has never run today, so it should still fire
        // (it missed its window). Actually — standard cron doesn't backfill.
        // We only fire if now is IN a matching minute. 15:00 doesn't match
        // "0 9 * * *", so it should NOT be due.
        let three_pm = 1_699_974_000u64; // 2023-11-14 15:00:00 UTC
        let job = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            true,
            0,
        );
        assert!(
            !is_job_due(&job, three_pm),
            "daily-at-9am job should NOT be due at 15:00 UTC"
        );
    }

    #[test]
    fn test_cron_expression_invalid_expression_not_due() {
        // An invalid cron expression should never fire.
        let job = make_job(
            CronSchedule::Cron {
                expression: "not a cron expr".to_string(),
            },
            true,
            0,
        );
        assert!(
            !is_job_due(&job, 1_700_000_000),
            "invalid cron expression should never fire"
        );
    }

    #[test]
    fn test_unsupported_schedule_reason_none_when_cron_implemented() {
        // After implementation, cron expressions are no longer unsupported.
        let enabled = make_job(
            CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            true,
            0,
        );
        assert_eq!(
            unsupported_schedule_reason(&enabled),
            None,
            "cron expression should no longer be unsupported"
        );
    }
}
