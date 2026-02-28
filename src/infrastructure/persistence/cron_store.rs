// JSON-based CronStore: persists cron jobs as a single JSON file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::domain::cron::{CronJob, CronSchedule, CronStore};
use crate::domain::error::DomainError;

/// File-based cron store. Jobs are stored in `<base_dir>/cron/jobs.json`.
///
/// All mutations are serialized via an internal `Mutex` to prevent
/// lost-update races when multiple async tasks share the same `Arc<FileCronStore>`.
#[derive(Debug)]
pub struct FileCronStore {
    path: PathBuf,
    /// Guards the read-modify-write cycle for all mutations.
    mu: Mutex<()>,
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
    #[serde(default)]
    last_run_at: u64,
}

impl FileCronStore {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            path: base_dir.as_ref().join("cron").join("jobs.json"),
            mu: Mutex::new(()),
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
        let parent = self
            .path
            .parent()
            .ok_or_else(|| DomainError::Other("cron jobs path has no parent".to_string()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| DomainError::Other(format!("failed to create cron dir: {}", e)))?;

        let file = CronFile {
            jobs: jobs.to_vec(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| DomainError::Other(format!("failed to serialize cron jobs: {}", e)))?;

        // Atomic write: write to a temp file in the same directory, then rename.
        // This avoids data loss if the process is killed mid-write.
        let tmp_path = parent.join(".jobs.json.tmp");
        std::fs::write(&tmp_path, json)
            .map_err(|e| DomainError::Other(format!("failed to write temp cron file: {}", e)))?;
        std::fs::rename(&tmp_path, &self.path)
            .map_err(|e| DomainError::Other(format!("failed to rename cron file: {}", e)))?;
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
        last_error: job.last_error.clone(),
        last_run_at: job.last_run_at,
    }
}

fn record_to_job(rec: CronJobRecord) -> Result<CronJob, DomainError> {
    let schedule = match rec.schedule_type.as_str() {
        "cron" => CronSchedule::Cron {
            expression: rec.cron_expression.unwrap_or_default(),
        },
        "interval" => CronSchedule::Interval {
            seconds: rec.interval_seconds.unwrap_or(3600),
        },
        other => {
            return Err(DomainError::Other(format!(
                "unknown schedule type '{}' for job '{}'",
                other, rec.name
            )));
        }
    };
    Ok(CronJob {
        id: rec.id,
        name: rec.name,
        message: rec.message,
        schedule,
        enabled: rec.enabled,
        deliver_to: rec.deliver_to,
        last_error: rec.last_error,
        last_run_at: rec.last_run_at,
    })
}

impl CronStore for FileCronStore {
    fn list(&self) -> Result<Vec<CronJob>, DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let records = self.load_all()?;
        records.into_iter().map(record_to_job).collect()
    }

    fn add(&self, job: CronJob) -> Result<(), DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let mut records = self.load_all()?;
        records.push(job_to_record(&job));
        self.save_all(&records)
    }

    fn remove(&self, id: &str) -> Result<(), DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let mut records = self.load_all()?;
        let before = records.len();
        records.retain(|r| r.id != id);
        if records.len() == before {
            return Err(DomainError::Other(format!(
                "cron job with id '{}' not found",
                id
            )));
        }
        self.save_all(&records)
    }

    fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let mut records = self.load_all()?;
        let rec = records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| DomainError::Other(format!("cron job with id '{}' not found", id)))?;
        rec.enabled = enabled;
        self.save_all(&records)
    }

    fn find_by_name(&self, name: &str) -> Result<Option<CronJob>, DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let records = self.load_all()?;
        match records.into_iter().find(|r| r.name == name) {
            Some(rec) => Ok(Some(record_to_job(rec)?)),
            None => Ok(None),
        }
    }

    fn set_last_error(&self, id: &str, error: Option<String>) -> Result<(), DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let mut records = self.load_all()?;
        let rec = records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| DomainError::Other(format!("cron job with id '{}' not found", id)))?;
        rec.last_error = error;
        self.save_all(&records)
    }

    fn set_last_run_at(&self, id: &str, timestamp: u64) -> Result<(), DomainError> {
        let _lock = self
            .mu
            .lock()
            .map_err(|e| DomainError::Other(format!("cron store lock poisoned: {}", e)))?;
        let mut records = self.load_all()?;
        let rec = records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| DomainError::Other(format!("cron job with id '{}' not found", id)))?;
        rec.last_run_at = timestamp;
        self.save_all(&records)
    }
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
            last_run_at: 0,
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
            last_run_at: 0,
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

        let found = store.find_by_name("Weather").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Weather");

        let not_found = store.find_by_name("Missing").unwrap();
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
