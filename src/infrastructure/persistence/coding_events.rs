//! Append-only JSONL event log persistence for coding jobs.
//!
//! Each job has its own event log file at `<jobs_dir>/<job_id>/events.jsonl`.
//! The event log is the source of truth. The `<jobs_dir>/index.json` snapshot
//! is rebuilt from logs on startup and only written periodically for fast
//! status queries.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::domain::coding_event::EventEnvelope;
use crate::domain::coding_job::JobState;
use crate::domain::coding_ports::{EventLogLine, EventLogStore};

/// Maximum event line size in bytes (1 MiB).
const MAX_EVENT_LINE_BYTES: usize = 1024 * 1024;

/// File-backed JSONL event log store.
///
/// Layout:
/// ```text
/// <jobs_dir>/
///   coordinator.lock
///   index.json
///   <job_id>/
///     events.jsonl
/// ```
#[derive(Debug)]
pub struct FileEventLogStore {
    jobs_dir: PathBuf,
}

impl FileEventLogStore {
    /// Create a new store rooted at `jobs_dir`.
    pub fn new(jobs_dir: PathBuf) -> Self {
        Self { jobs_dir }
    }

    /// Returns the path to the event log for a job.
    fn log_path(&self, job_id: &str) -> PathBuf {
        self.jobs_dir.join(job_id).join("events.jsonl")
    }

    /// Returns the path to the jobs index snapshot.
    fn index_path(&self) -> PathBuf {
        self.jobs_dir.join("index.json")
    }

    /// Returns the path to the coordinator lock file.
    fn lock_path(&self) -> PathBuf {
        self.jobs_dir.join("coordinator.lock")
    }
}

impl EventLogStore for FileEventLogStore {
    fn discover_jobs(&self) -> Vec<String> {
        let mut jobs = Vec::new();
        let entries = match fs::read_dir(&self.jobs_dir) {
            Ok(e) => e,
            Err(_) => return jobs,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let events_path = path.join("events.jsonl");
                if events_path.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        jobs.push(name.to_string());
                    }
                }
            }
        }
        jobs.sort();
        jobs
    }

    fn read_log(&self, job_id: &str) -> Vec<EventLogLine> {
        let path = self.log_path(job_id);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        let mut lines = Vec::new();
        for (idx, line_result) in reader.lines().enumerate() {
            let line_number = idx + 1;
            match line_result {
                Ok(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<EventEnvelope>(trimmed) {
                        Ok(env) => lines.push(EventLogLine::Valid(env)),
                        Err(_) => lines.push(EventLogLine::Corrupt {
                            line_number,
                            raw: raw.clone(),
                        }),
                    }
                }
                Err(_) => {
                    lines.push(EventLogLine::Corrupt {
                        line_number,
                        raw: String::new(),
                    });
                }
            }
        }
        lines
    }

    fn append_event(&mut self, job_id: &str, event: &EventEnvelope) {
        let path = self.log_path(job_id);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let serialized = match serde_json::to_string(event) {
            Ok(s) => s,
            Err(_) => return,
        };
        // Enforce 1 MiB line limit
        if serialized.len() > MAX_EVENT_LINE_BYTES {
            tracing::warn!(
                job_id = job_id,
                size = serialized.len(),
                "event line exceeds 1 MiB limit, skipping"
            );
            return;
        }
        let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let _ = writeln!(file, "{}", serialized);
        // fsync for durability
        let _ = file.flush();
        let _ = file.sync_all();
    }

    fn write_index(&mut self, entries: &[(String, JobState)]) {
        let map: HashMap<&str, &str> = entries
            .iter()
            .map(|(id, state)| {
                let state_str = match state {
                    JobState::Queued => "queued",
                    JobState::Preparing => "preparing",
                    JobState::Running => "running",
                    JobState::Blocked => "blocked",
                    JobState::Failed => "failed",
                    JobState::Succeeded => "succeeded",
                    JobState::Canceled => "canceled",
                };
                (id.as_str(), state_str)
            })
            .collect();
        let path = self.index_path();
        let _ = fs::create_dir_all(&self.jobs_dir);
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let _ = fs::write(&path, json);
        }
    }

    fn try_acquire_lock(&self) -> bool {
        let path = self.lock_path();
        let _ = fs::create_dir_all(&self.jobs_dir);

        // Remove stale lock from a dead process
        if let Ok(contents) = fs::read_to_string(&path) {
            let pid_str = contents.trim();
            if let Ok(pid) = pid_str.parse::<u32>() {
                let proc_path = format!("/proc/{}", pid);
                if Path::new(&proc_path).exists() {
                    return false; // Lock held by a live process
                }
                // Stale lock — remove before attempting atomic create
                let _ = fs::remove_file(&path);
            }
        }

        // Atomic create via O_CREAT|O_EXCL — avoids TOCTOU race
        let pid = std::process::id();
        let result = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match result {
            Ok(mut f) => {
                use std::io::Write;
                let _ = write!(f, "{}", pid);
                true
            }
            Err(_) => false, // Another process won the race
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coding_event::EventSource;

    fn make_envelope(job_id: &str, event_type: &str, seq: u64) -> EventEnvelope {
        EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            run_id: "run_000001".to_string(),
            job_id: job_id.to_string(),
            source: EventSource::Coordinator,
            event_type: event_type.to_string(),
            seq,
            payload: serde_json::json!({"test": true}),
        }
    }

    #[test]
    fn test_append_and_read_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        let event = make_envelope("job_001", "job.start", 1);
        store.append_event("job_001", &event);

        let lines = store.read_log("job_001");
        assert_eq!(lines.len(), 1);
        match &lines[0] {
            EventLogLine::Valid(env) => {
                assert_eq!(env.job_id, "job_001");
                assert_eq!(env.event_type, "job.start");
                assert_eq!(env.seq, 1);
            }
            EventLogLine::Corrupt { .. } => panic!("expected valid line"),
        }
    }

    #[test]
    fn test_multiple_events_append_sequentially() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        store.append_event("job_001", &make_envelope("job_001", "job.start", 1));
        store.append_event("job_001", &make_envelope("job_001", "job.ready", 2));
        store.append_event("job_001", &make_envelope("job_001", "job.status", 3));

        let lines = store.read_log("job_001");
        assert_eq!(lines.len(), 3);
        for (i, line) in lines.iter().enumerate() {
            match line {
                EventLogLine::Valid(env) => assert_eq!(env.seq, (i + 1) as u64),
                EventLogLine::Corrupt { .. } => panic!("expected valid line at {}", i),
            }
        }
    }

    #[test]
    fn test_discover_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        store.append_event("job_001", &make_envelope("job_001", "job.start", 1));
        store.append_event("job_003", &make_envelope("job_003", "job.start", 1));
        // job_002 has dir but no events.jsonl
        fs::create_dir_all(dir.path().join("job_002")).unwrap();

        let jobs = store.discover_jobs();
        assert_eq!(jobs, vec!["job_001", "job_003"]);
    }

    #[test]
    fn test_read_log_nonexistent_job() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileEventLogStore::new(dir.path().to_path_buf());
        let lines = store.read_log("nonexistent");
        assert!(lines.is_empty());
    }

    #[test]
    fn test_corrupted_line_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let job_dir = dir.path().join("job_001");
        fs::create_dir_all(&job_dir).unwrap();

        let mut store = FileEventLogStore::new(dir.path().to_path_buf());
        store.append_event("job_001", &make_envelope("job_001", "job.start", 1));

        // Append a corrupted line manually
        let log_path = job_dir.join("events.jsonl");
        let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
        writeln!(file, "{{invalid json").unwrap();

        store.append_event("job_001", &make_envelope("job_001", "job.ready", 2));

        let lines = store.read_log("job_001");
        assert_eq!(lines.len(), 3);
        assert!(matches!(&lines[0], EventLogLine::Valid(_)));
        assert!(matches!(
            &lines[1],
            EventLogLine::Corrupt { line_number: 2, .. }
        ));
        assert!(matches!(&lines[2], EventLogLine::Valid(_)));
    }

    #[test]
    fn test_truncated_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let job_dir = dir.path().join("job_001");
        fs::create_dir_all(&job_dir).unwrap();

        let mut store = FileEventLogStore::new(dir.path().to_path_buf());
        store.append_event("job_001", &make_envelope("job_001", "job.start", 1));

        // Append a truncated line (no newline, partial JSON)
        let log_path = job_dir.join("events.jsonl");
        let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
        write!(file, r#"{{"v":"1.0","ts":"2026"#).unwrap();

        let lines = store.read_log("job_001");
        assert_eq!(lines.len(), 2);
        assert!(matches!(&lines[0], EventLogLine::Valid(_)));
        assert!(matches!(
            &lines[1],
            EventLogLine::Corrupt { line_number: 2, .. }
        ));
    }

    #[test]
    fn test_write_and_read_index() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        store.write_index(&[
            ("job_001".to_string(), JobState::Succeeded),
            ("job_002".to_string(), JobState::Running),
        ]);

        let content = fs::read_to_string(dir.path().join("index.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["job_001"], "succeeded");
        assert_eq!(parsed["job_002"], "running");
    }

    #[test]
    fn test_try_acquire_lock_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileEventLogStore::new(dir.path().to_path_buf());
        assert!(store.try_acquire_lock());
        // Lock file should contain our PID
        let content = fs::read_to_string(dir.path().join("coordinator.lock")).unwrap();
        assert_eq!(content.trim(), std::process::id().to_string());
    }

    #[test]
    fn test_try_acquire_lock_stale_pid() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("coordinator.lock");
        fs::create_dir_all(dir.path()).unwrap();
        // Write a PID that doesn't exist (very high number)
        fs::write(&lock_path, "9999999").unwrap();

        let store = FileEventLogStore::new(dir.path().to_path_buf());
        assert!(store.try_acquire_lock());
    }

    #[test]
    fn test_try_acquire_lock_live_pid() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("coordinator.lock");
        fs::create_dir_all(dir.path()).unwrap();
        // Write our own PID (definitely alive)
        fs::write(&lock_path, std::process::id().to_string()).unwrap();

        let store = FileEventLogStore::new(dir.path().to_path_buf());
        assert!(!store.try_acquire_lock());
    }

    #[test]
    fn test_oversized_event_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        let big_payload = "x".repeat(2 * 1024 * 1024);
        let event = EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            run_id: "run_001".to_string(),
            job_id: "job_001".to_string(),
            source: EventSource::Coordinator,
            event_type: "log.message".to_string(),
            seq: 1,
            payload: serde_json::json!({"message": big_payload}),
        };
        store.append_event("job_001", &event);

        let lines = store.read_log("job_001");
        assert!(lines.is_empty(), "oversized event should not be persisted");
    }

    #[test]
    fn test_envelope_fields_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        let event = EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-02-25T12:00:00Z".to_string(),
            run_id: "run_xyz".to_string(),
            job_id: "job_abc".to_string(),
            source: EventSource::Worker,
            event_type: "tool.result".to_string(),
            seq: 42,
            payload: serde_json::json!({"ok": true, "tool": "read_file"}),
        };
        store.append_event("job_abc", &event);

        let lines = store.read_log("job_abc");
        assert_eq!(lines.len(), 1);
        match &lines[0] {
            EventLogLine::Valid(env) => {
                assert_eq!(env.v, "1.0");
                assert_eq!(env.ts, "2026-02-25T12:00:00Z");
                assert_eq!(env.run_id, "run_xyz");
                assert_eq!(env.job_id, "job_abc");
                assert_eq!(env.event_type, "tool.result");
                assert_eq!(env.seq, 42);
                assert_eq!(env.payload["ok"], true);
            }
            EventLogLine::Corrupt { .. } => panic!("expected valid line"),
        }
    }

    #[test]
    fn test_empty_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let job_dir = dir.path().join("job_001");
        fs::create_dir_all(&job_dir).unwrap();
        fs::write(job_dir.join("events.jsonl"), "").unwrap();

        let store = FileEventLogStore::new(dir.path().to_path_buf());
        let lines = store.read_log("job_001");
        assert!(lines.is_empty());
    }

    #[test]
    fn test_index_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = FileEventLogStore::new(dir.path().to_path_buf());

        store.write_index(&[("job_001".to_string(), JobState::Running)]);
        store.write_index(&[("job_001".to_string(), JobState::Succeeded)]);

        let content = fs::read_to_string(dir.path().join("index.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["job_001"], "succeeded");
    }
}
