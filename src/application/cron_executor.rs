// Cron tick executor: checks due jobs and sends them through the agent.

use std::time::Duration;

use crate::domain::agent::AgentLoop;
use crate::domain::cron::{
    CronJob, CronJobResult, CronStore, is_job_due, unsupported_schedule_reason,
};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};

/// Execute a single cron tick: list all jobs and run those that are due.
///
/// Returns a list of results for each executed job.
/// Disabled and not-yet-due jobs are silently skipped.
pub async fn execute_cron_tick(
    store: &dyn CronStore,
    agent: &dyn AgentLoop,
    timeout: Duration,
) -> Result<Vec<CronJobResult>, DomainError> {
    let jobs = store.list()?;
    let now_secs = now_unix_secs();
    let mut results = Vec::new();

    for job in &jobs {
        if handle_unsupported_job(store, job) {
            continue;
        }

        if !is_job_due(job, now_secs) {
            continue;
        }

        record_last_run(store, &job.id, now_secs);

        let result = execute_single_job(agent, job, timeout).await;
        record_job_error(store, &job.id, &result);

        results.push(result);
    }

    Ok(results)
}

/// Current time as Unix seconds.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn handle_unsupported_job(store: &dyn CronStore, job: &CronJob) -> bool {
    let Some(reason) = unsupported_schedule_reason(job) else {
        return false;
    };

    if job.last_error.as_deref() == Some(reason) {
        return true;
    }

    if let Err(e) = store.set_last_error(&job.id, Some(reason.to_string())) {
        tracing::warn!(
            job_id = %job.id,
            "failed to update last_error on unsupported cron expression job: {}",
            e
        );
    }
    true
}

fn record_last_run(store: &dyn CronStore, job_id: &str, now_secs: u64) {
    if let Err(e) = store.set_last_run_at(job_id, now_secs) {
        tracing::warn!(job_id = %job_id, "failed to update last_run_at: {}", e);
    }
}

fn record_job_error(store: &dyn CronStore, job_id: &str, result: &CronJobResult) {
    let error_val = if result.ok {
        None
    } else {
        Some(result.response.clone())
    };
    if let Err(e) = store.set_last_error(job_id, error_val) {
        tracing::warn!(
            job_id = %job_id,
            "failed to update last_error on cron job: {}",
            e
        );
    }
}

/// Execute a single cron job with a timeout.
async fn execute_single_job(
    agent: &dyn AgentLoop,
    job: &crate::domain::cron::CronJob,
    timeout: Duration,
) -> CronJobResult {
    let mut messages = vec![Message {
        role: Role::User,
        content: job.message.clone(),
        tool_calls: vec![],
        tool_call_id: None,
    }];

    let result = tokio::time::timeout(timeout, agent.process(&mut messages)).await;

    match result {
        Ok(Ok(agent_result)) => CronJobResult {
            job_id: job.id.clone(),
            response: agent_result.response,
            ok: true,
            deliver_to: job.deliver_to.clone(),
        },
        Ok(Err(e)) => CronJobResult {
            job_id: job.id.clone(),
            response: format!("error: {}", e),
            ok: false,
            deliver_to: job.deliver_to.clone(),
        },
        Err(_) => CronJobResult {
            job_id: job.id.clone(),
            response: "timeout".to_string(),
            ok: false,
            deliver_to: job.deliver_to.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentInfo, AgentResult};
    use crate::domain::cron::{CronJob, CronSchedule};
    use std::future::Future;
    use std::pin::Pin;

    // -- Mock agent --

    struct MockAgent {
        response: String,
    }

    impl AgentLoop for MockAgent {
        fn process<'a>(
            &'a self,
            _messages: &'a mut Vec<Message>,
        ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
            let resp = self.response.clone();
            Box::pin(async move { Ok(AgentResult::text(resp)) })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    // -- Slow agent (always exceeds timeout) --

    struct SlowAgent;

    impl AgentLoop for SlowAgent {
        fn process<'a>(
            &'a self,
            _messages: &'a mut Vec<Message>,
        ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(AgentResult::text("done"))
            })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    // -- Mock CronStore --

    struct MockCronStore {
        jobs: std::sync::Mutex<Vec<CronJob>>,
    }

    impl MockCronStore {
        fn new(jobs: Vec<CronJob>) -> Self {
            Self {
                jobs: std::sync::Mutex::new(jobs),
            }
        }
    }

    impl CronStore for MockCronStore {
        fn list(&self) -> Result<Vec<CronJob>, DomainError> {
            Ok(self.jobs.lock().unwrap().clone())
        }
        fn add(&self, job: CronJob) -> Result<(), DomainError> {
            self.jobs.lock().unwrap().push(job);
            Ok(())
        }
        fn remove(&self, id: &str) -> Result<(), DomainError> {
            self.jobs.lock().unwrap().retain(|j| j.id != id);
            Ok(())
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError> {
            if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
                j.enabled = enabled;
            }
            Ok(())
        }
        fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError> {
            Ok(self
                .jobs
                .lock()
                .unwrap()
                .iter()
                .find(|j| j.name == name)
                .cloned())
        }
        fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError> {
            if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
                j.last_error = error;
            }
            Ok(())
        }
        fn set_last_run_at(&self, id: &str, timestamp: u64) -> Result<(), DomainError> {
            if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
                j.last_run_at = timestamp;
            }
            Ok(())
        }
    }

    fn make_job(name: &str, enabled: bool) -> CronJob {
        CronJob {
            id: name.to_lowercase(),
            name: name.to_string(),
            message: format!("Run {}", name),
            schedule: CronSchedule::Interval { seconds: 60 },
            enabled,
            deliver_to: None,
            last_error: None,
            last_run_at: 0,
        }
    }

    #[tokio::test]
    async fn test_execute_enabled_jobs() {
        let store = MockCronStore::new(vec![make_job("weather", true)]);
        let agent = MockAgent {
            response: "Sunny".to_string(),
        };
        let results = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].ok);
        assert_eq!(results[0].response, "Sunny");
    }

    #[tokio::test]
    async fn test_skip_disabled_jobs() {
        let store = MockCronStore::new(vec![make_job("weather", false)]);
        let agent = MockAgent {
            response: "Sunny".to_string(),
        };
        let results = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_timeout_records_error() {
        let store = MockCronStore::new(vec![make_job("slow", true)]);
        let agent = SlowAgent;
        let results = execute_cron_tick(&store, &agent, Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].ok);
        assert_eq!(results[0].response, "timeout");
        // Check that last_error was recorded on the job.
        let job = store.find_by_name("slow").unwrap().unwrap();
        assert_eq!(job.last_error.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn test_deliver_to_propagated() {
        let mut job = make_job("report", true);
        job.deliver_to = Some("telegram:12345".to_string());
        let store = MockCronStore::new(vec![job]);
        let agent = MockAgent {
            response: "Report done".to_string(),
        };
        let results = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].deliver_to.as_deref(), Some("telegram:12345"));
    }

    #[tokio::test]
    async fn test_success_clears_last_error() {
        let mut job = make_job("recovery", true);
        job.last_error = Some("previous error".to_string());
        let store = MockCronStore::new(vec![job]);
        let agent = MockAgent {
            response: "OK".to_string(),
        };
        let _ = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();
        let job = store.find_by_name("recovery").unwrap().unwrap();
        assert!(job.last_error.is_none());
    }

    #[tokio::test]
    async fn test_cron_expression_jobs_are_skipped_with_error_marker() {
        let job = CronJob {
            id: "morning-brief".to_string(),
            name: "Morning Brief".to_string(),
            message: "Good morning brief".to_string(),
            schedule: CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            enabled: true,
            deliver_to: None,
            last_error: None,
            last_run_at: 0,
        };
        let store = MockCronStore::new(vec![job]);
        let agent = MockAgent {
            response: "should not run".to_string(),
        };

        let results = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();

        assert!(results.is_empty());
        let stored = store.find_by_name("Morning Brief").unwrap().unwrap();
        assert!(
            stored
                .last_error
                .as_deref()
                .is_some_and(|e| e.contains("not implemented"))
        );
    }

    #[tokio::test]
    async fn test_disabled_cron_expression_jobs_are_silently_skipped() {
        let job = CronJob {
            id: "disabled-brief".to_string(),
            name: "Disabled Brief".to_string(),
            message: "Should not run".to_string(),
            schedule: CronSchedule::Cron {
                expression: "0 9 * * *".to_string(),
            },
            enabled: false,
            deliver_to: None,
            last_error: None,
            last_run_at: 0,
        };
        let store = MockCronStore::new(vec![job]);
        let agent = MockAgent {
            response: "should not run".to_string(),
        };

        let results = execute_cron_tick(&store, &agent, Duration::from_secs(60))
            .await
            .unwrap();

        assert!(results.is_empty());
        let stored = store.find_by_name("Disabled Brief").unwrap().unwrap();
        assert!(stored.last_error.is_none());
    }
}
