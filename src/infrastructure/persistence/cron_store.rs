// JSON-based CronStore: persists cron jobs as a single JSON file.

use std::path::{Path, PathBuf};

use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::domain::error::DomainError;

/// File-based cron store. Jobs are stored in `<base_dir>/cron/jobs.json`.
#[derive(Debug)]
pub struct FileCronStore {
    path: PathBuf,
}

// -- Serializable structs --

#[derive(serde::Serialize, serde::Deserialize)]
struct CronFile {
    jobs: Vec<CronJobRecord>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CronJobRecord {
    id: String,
    name: String,
    message: String,
    schedule_type: String, // "interval" or "cron"
    #[serde(default)]
    interval_seconds: Option<u64>,
    #[serde(default)]
    cron_expression: Option<String>,
    enabled: bool,
    #[serde(default)]
    deliver_to: Option<String>,
    #[serde(default)]
    last_error: Option<String>,
}

impl FileCronStore {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            path: base_dir.as_ref().join("cron").join("jobs.json"),
        }
    }

    fn load_all(&self) -> Result<Vec<CronJobRecord>, DomainError> {
        if !self.path.exists() {
            return Ok(vec![]);
        }
        let data = std::fs::read_to_string(&self.path)
            .map_err(|e| DomainError::Other(format!("failed to read cron jobs: {}", e)))?;
        let file: CronFile = serde_json::from_str(&data)
            .map_err(|e| DomainError::Other(format!("failed to parse cron jobs: {}", e)))?;
        Ok(file.jobs)
    }

    fn save_all(&self, jobs: &[CronJobRecord]) -> Result<(), DomainError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Other(format!("failed to create cron dir: {}", e)))?;
        }
        let file = CronFile {
            jobs: jobs.to_vec(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| DomainError::Other(format!("failed to serialize cron jobs: {}", e)))?;
        std::fs::write(&self.path, json)
            .map_err(|e| DomainError::Other(format!("failed to write cron jobs: {}", e)))?;
        Ok(())
    }
}

fn job_to_record(job: &CronJob) -> CronJobRecord {
    let (schedule_type, interval_seconds, cron_expression) = match &job.schedule {
        CronSchedule::Interval { seconds } => ("interval".to_string(), Some(*seconds), None),
        CronSchedule::Cron { expression } => ("cron".to_string(), None, Some(expression.clone())),
    };
    CronJobRecord {
        id: job.id.clone(),
        name: job.name.clone(),
        message: job.message.clone(),
        schedule_type,
        interval_seconds,
        cron_expression,
        enabled: job.enabled,
        deliver_to: job.deliver_to.clone(),
        last_error: None,
    }
}

fn record_to_job(rec: CronJobRecord) -> CronJob {
    let schedule = match rec.schedule_type.as_str() {
        "cron" => CronSchedule::Cron {
            expression: rec.cron_expression.unwrap_or_default(),
        },
        _ => CronSchedule::Interval {
            seconds: rec.interval_seconds.unwrap_or(3600),
        },
    };
    CronJob {
        id: rec.id,
        name: rec.name,
        message: rec.message,
        schedule,
        enabled: rec.enabled,
        deliver_to: rec.deliver_to,
        last_error: rec.last_error,
    }
}

impl CronStore for FileCronStore {
    fn list(&self) -> Result<Vec<CronJob>, DomainError> {
        let records = self.load_all()?;
        Ok(records.into_iter().map(record_to_job).collect())
    }

    fn add(&self, job: CronJob) -> Result<(), DomainError> {
        let mut records = self.load_all()?;
        records.push(job_to_record(&job));
        self.save_all(&records)
    }

    fn remove(&self, id: &str) -> Result<(), DomainError> {
        let mut records = self.load_all()?;
        records.retain(|r| r.id != id);
        self.save_all(&records)
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError> {
        let mut records = self.load_all()?;
        if let Some(rec) = records.iter_mut().find(|r| r.id == id) {
            rec.enabled = enabled;
        }
        self.save_all(&records)
    }

    fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError> {
        let records = self.load_all()?;
        Ok(records
            .into_iter()
            .find(|r| r.name == name)
            .map(record_to_job))
    }

    fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError> {
        let mut records = self.load_all()?;
        if let Some(rec) = records.iter_mut().find(|r| r.id == id) {
            rec.last_error = error;
        }
        self.save_all(&records)
    }
}

/// Helper: find a job by name from the store.
pub fn find_by_name(store: &dyn CronStore, name: &str) -> Result<Option<CronJob>, DomainError> {
    store.find_by_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_interval_job(name: &str, seconds: u64) -> CronJob {
        CronJob {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            message: format!("Run {}", name),
            schedule: CronSchedule::Interval { seconds },
            enabled: true,
            deliver_to: None,
            last_error: None,
        }
    }

    fn make_cron_job(name: &str, expr: &str) -> CronJob {
        CronJob {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            message: format!("Run {}", name),
            schedule: CronSchedule::Cron {
                expression: expr.to_string(),
            },
            enabled: true,
            deliver_to: None,
            last_error: None,
        }
    }

    #[test]
    fn test_add_and_list() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        store.add(make_interval_job("Weather", 3600)).unwrap();
        let jobs = store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "Weather");
    }

    #[test]
    fn test_add_cron_expression() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        store.add(make_cron_job("Brief", "0 9 * * *")).unwrap();
        let jobs = store.list().unwrap();
        assert_eq!(jobs.len(), 1);
        match &jobs[0].schedule {
            CronSchedule::Cron { expression } => assert_eq!(expression, "0 9 * * *"),
            _ => panic!("expected cron schedule"),
        }
    }

    #[test]
    fn test_remove() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        let job = make_interval_job("Weather", 3600);
        let job_id = job.id.clone();
        store.add(job).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);

        store.remove(&job_id).unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
    }

    #[test]
    fn test_disable_and_enable() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        let job = make_interval_job("Weather", 3600);
        let job_id = job.id.clone();
        store.add(job).unwrap();

        store.set_enabled(&job_id, false).unwrap();
        let jobs = store.list().unwrap();
        assert!(!jobs[0].enabled);

        store.set_enabled(&job_id, true).unwrap();
        let jobs = store.list().unwrap();
        assert!(jobs[0].enabled);
    }

    #[test]
    fn test_persistence_across_instances() {
        let tmp = TempDir::new().unwrap();

        let store1 = FileCronStore::new(tmp.path());
        store1.add(make_interval_job("Weather", 3600)).unwrap();

        let store2 = FileCronStore::new(tmp.path());
        let jobs = store2.list().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name, "Weather");
    }

    #[test]
    fn test_empty_store() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        let jobs = store.list().unwrap();
        assert!(jobs.is_empty());
    }

    #[test]
    fn test_find_by_name() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        store.add(make_interval_job("Weather", 3600)).unwrap();
        store.add(make_cron_job("Brief", "0 9 * * *")).unwrap();

        let found = find_by_name(&store, "Weather").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Weather");

        let not_found = find_by_name(&store, "Missing").unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_multiple_jobs() {
        let tmp = TempDir::new().unwrap();
        let store = FileCronStore::new(tmp.path());

        store.add(make_interval_job("Job1", 60)).unwrap();
        store.add(make_interval_job("Job2", 120)).unwrap();
        store.add(make_cron_job("Job3", "0 * * * *")).unwrap();

        assert_eq!(store.list().unwrap().len(), 3);
    }
}
