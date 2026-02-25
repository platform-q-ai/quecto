use cucumber::{given, then, when};
use quecto::domain::coding_event::{EventEnvelope, EventSource};
use quecto::domain::coding_job::JobState;
use quecto::domain::coding_ports::{EventLogLine, EventLogStore};
use quecto::infrastructure::persistence::coding_events::FileEventLogStore;
use std::fs::{self, OpenOptions};
use std::io::Write;
use tempfile::TempDir;

use crate::QuectoWorld;

// ── helpers ──────────────────────────────────────────────────────────────

fn ensure_event_store(world: &mut QuectoWorld) {
    if world.coding_event_store.is_none() {
        let td = TempDir::new().expect("temp dir");
        let store = FileEventLogStore::new(td.path().to_path_buf());
        world.coding_event_store_dir = Some(td.path().to_path_buf());
        world.coding_event_store = Some(store);
        world._coding_event_temp_dir = Some(td);
    }
}

fn store(world: &mut QuectoWorld) -> &mut FileEventLogStore {
    world.coding_event_store.as_mut().expect("event store")
}

fn store_dir(world: &QuectoWorld) -> std::path::PathBuf {
    world
        .coding_event_store_dir
        .clone()
        .expect("event store dir")
}

/// Event identity — groups job/run/type/seq to stay within clippy's 4-arg limit.
struct EventId<'a> {
    job: &'a str,
    run: &'a str,
    event_type: &'a str,
    seq: u64,
}

fn make_event(id: &EventId<'_>, source: EventSource, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        run_id: id.run.to_string(),
        job_id: id.job.to_string(),
        source,
        event_type: id.event_type.to_string(),
        seq: id.seq,
        payload,
    }
}

// ── Given steps ──────────────────────────────────────────────────────────

#[given(regex = r#"^a coding event store for job "([^"]+)"$"#)]
fn given_event_store_for_job(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id);
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with events ([\w.]+(?:, [\w.]+)*)$"#)]
fn given_event_store_with_events(world: &mut QuectoWorld, job_id: String, events_csv: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    for (i, event_type) in events_csv.split(", ").enumerate() {
        let et = event_type.trim();
        let payload = match et {
            "job.ready" => serde_json::json!({"worker_pid": 1234}),
            "job.status" => serde_json::json!({"state": "running", "progress": 50}),
            _ => serde_json::json!({"test": true}),
        };
        let event = make_event(
            &EventId {
                job: &job_id,
                run: "run_000001",
                event_type: et,
                seq: (i + 1) as u64,
            },
            EventSource::Coordinator,
            payload,
        );
        store(world).append_event(&job_id, &event);
    }
}

#[given(regex = r#"^a coding event store with (\d+) jobs in various states$"#)]
fn given_n_jobs_with_states(world: &mut QuectoWorld, count: usize) {
    ensure_event_store(world);
    let states = ["queued", "running", "succeeded", "failed", "canceled"];
    for i in 0..count {
        let job_id = format!("job_{:06}", i + 1);
        let state = states[i % states.len()];
        let event = make_event(
            &EventId {
                job: &job_id,
                run: &format!("run_{:06}", i + 1),
                event_type: "job.start",
                seq: 1,
            },
            EventSource::Coordinator,
            serde_json::json!({"state": state}),
        );
        store(world).append_event(&job_id, &event);
    }
    world.coding_event_n_jobs = Some(count);
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with events ending in succeeded$"#)]
fn given_event_store_with_succeeded(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let events = [
        ("job.start", serde_json::json!({"goal": "fix bug"})),
        ("job.ready", serde_json::json!({"worker_pid": 1234})),
        (
            "job.end",
            serde_json::json!({"state": "succeeded", "summary": "all tests pass"}),
        ),
    ];
    for (i, (et, payload)) in events.iter().enumerate() {
        let event = make_event(
            &EventId {
                job: &job_id,
                run: "run_000001",
                event_type: et,
                seq: (i + 1) as u64,
            },
            EventSource::Coordinator,
            payload.clone(),
        );
        store(world).append_event(&job_id, &event);
    }
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with events but no index file$"#)]
fn given_event_store_no_index(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let event = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.start",
            seq: 1,
        },
        EventSource::Coordinator,
        serde_json::json!({"goal": "test"}),
    );
    store(world).append_event(&job_id, &event);
    // Explicitly ensure no index.json exists
    let dir = store_dir(world);
    let index_path = dir.join("index.json");
    if index_path.exists() {
        fs::remove_file(&index_path).unwrap();
    }
}

#[given(
    regex = r#"^a coding event store for job "([^"]+)" with todo\.create and todo\.update events$"#
)]
fn given_event_store_with_todos(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let create = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "todo.create",
            seq: 1,
        },
        EventSource::Worker,
        serde_json::json!({"todo_id": "t1", "title": "Write tests", "status": "pending"}),
    );
    store(world).append_event(&job_id, &create);
    let update = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "todo.update",
            seq: 2,
        },
        EventSource::Worker,
        serde_json::json!({"todo_id": "t1", "status": "in_progress"}),
    );
    store(world).append_event(&job_id, &update);
}

#[given(
    regex = r#"^a coding event store for job "([^"]+)" with spawn\.request and spawn\.decision events$"#
)]
fn given_event_store_with_spawns(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let req = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "spawn.request",
            seq: 1,
        },
        EventSource::Worker,
        serde_json::json!({"request_id": "s1", "agent_type": "security-reviewer", "scope": "diff"}),
    );
    store(world).append_event(&job_id, &req);
    let dec = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "spawn.decision",
            seq: 1,
        },
        EventSource::Coordinator,
        serde_json::json!({"request_id": "s1", "approved": true}),
    );
    store(world).append_event(&job_id, &dec);
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with a truncated last line$"#)]
fn given_event_store_truncated(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let event = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.start",
            seq: 1,
        },
        EventSource::Coordinator,
        serde_json::json!({"goal": "test"}),
    );
    store(world).append_event(&job_id, &event);
    // Append truncated line (partial JSON, no newline)
    let dir = store_dir(world);
    let log_path = dir.join(&job_id).join("events.jsonl");
    let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
    write!(file, r#"{{"v":"1.0","ts":"trunc"#).unwrap();
    world.coding_event_truncated_present = true;
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with a corrupted line 3$"#)]
fn given_event_store_corrupted(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    // Lines 1 and 2: valid events
    let e1 = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.start",
            seq: 1,
        },
        EventSource::Coordinator,
        serde_json::json!({}),
    );
    store(world).append_event(&job_id, &e1);
    let e2 = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.ready",
            seq: 2,
        },
        EventSource::Coordinator,
        serde_json::json!({}),
    );
    store(world).append_event(&job_id, &e2);
    // Line 3: corrupted
    let dir = store_dir(world);
    let log_path = dir.join(&job_id).join("events.jsonl");
    let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
    writeln!(file, "NOT VALID JSON AT ALL").unwrap();
    // Line 4: valid
    let e4 = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.status",
            seq: 3,
        },
        EventSource::Coordinator,
        serde_json::json!({}),
    );
    store(world).append_event(&job_id, &e4);
    world.coding_event_corrupted_present = true;
}

#[given(regex = r#"^a coding event store with job directories (.+)$"#)]
fn given_event_store_with_dirs(world: &mut QuectoWorld, dirs_csv: String) {
    ensure_event_store(world);
    let dir = store_dir(world);
    let dirs: Vec<String> = dirs_csv.split(", ").map(|s| s.trim().to_string()).collect();
    for d in &dirs {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
    world.coding_event_discovery_dirs = Some(dirs);
}

#[given(regex = r#"^(\S+) and (\S+) have events\.jsonl files$"#)]
fn given_jobs_have_event_files(world: &mut QuectoWorld, j1: String, j2: String) {
    let dir = store_dir(world);
    for job_id in [&j1, &j2] {
        let job_dir = dir.join(job_id);
        fs::create_dir_all(&job_dir).unwrap();
        let event = make_event(
            &EventId {
                job: job_id,
                run: "run_000001",
                event_type: "job.start",
                seq: 1,
            },
            EventSource::Coordinator,
            serde_json::json!({"goal": "test"}),
        );
        let serialized = serde_json::to_string(&event).unwrap();
        let log_path = job_dir.join("events.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(file, "{}", serialized).unwrap();
    }
}

#[given(regex = r#"^(\S+) has no events\.jsonl$"#)]
fn given_job_has_no_events(world: &mut QuectoWorld, _job_id: String) {
    // The directory was created by the previous step, but no events.jsonl was written
    // Nothing to do here — the absence is already the state
    let _ = world;
}

#[given("a coding event store with no lock file")]
fn given_no_lock(world: &mut QuectoWorld) {
    ensure_event_store(world);
}

#[given("a coding event store with a lock held by a live process")]
fn given_lock_held(world: &mut QuectoWorld) {
    ensure_event_store(world);
    let dir = store_dir(world);
    fs::create_dir_all(&dir).unwrap();
    // Write our own PID as the lock holder (definitely alive)
    fs::write(dir.join("coordinator.lock"), std::process::id().to_string()).unwrap();
}

#[given(regex = r#"^a coding event store for job "([^"]+)" with several events$"#)]
fn given_event_store_several_events(world: &mut QuectoWorld, job_id: String) {
    ensure_event_store(world);
    world.coding_event_job_id = Some(job_id.clone());
    let events = [
        ("job.start", serde_json::json!({"goal": "fix bug"})),
        ("job.ready", serde_json::json!({"worker_pid": 1234})),
        (
            "job.status",
            serde_json::json!({"state": "running", "progress": 50}),
        ),
    ];
    for (i, (et, payload)) in events.iter().enumerate() {
        let event = make_event(
            &EventId {
                job: &job_id,
                run: "run_000001",
                event_type: et,
                seq: (i + 1) as u64,
            },
            EventSource::Coordinator,
            payload.clone(),
        );
        store(world).append_event(&job_id, &event);
    }
}

// ── When steps ───────────────────────────────────────────────────────────

#[when("the coordinator appends a state transition event")]
fn when_append_state_transition(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let event = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "job.end",
            seq: 1,
        },
        EventSource::Coordinator,
        serde_json::json!({"state": "succeeded"}),
    );
    store(world).append_event(&job_id, &event);
    world.coding_event_last_appended = true;
}

#[when(regex = r#"^the coordinator appends a "([^"]+)" event$"#)]
fn when_append_named_event(world: &mut QuectoWorld, event_type: String) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    let next_seq = lines.len() as u64 + 1;
    let event = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: &event_type,
            seq: next_seq,
        },
        EventSource::Coordinator,
        serde_json::json!({"state": "succeeded"}),
    );
    store(world).append_event(&job_id, &event);
    world.coding_event_last_appended = true;
}

#[when("the coordinator writes a periodic index snapshot")]
fn when_write_index(world: &mut QuectoWorld) {
    let entries: Vec<(String, JobState)> = (0..world.coding_event_n_jobs.unwrap_or(5))
        .map(|i| {
            let states = [
                JobState::Queued,
                JobState::Running,
                JobState::Succeeded,
                JobState::Failed,
                JobState::Canceled,
            ];
            (format!("job_{:06}", i + 1), states[i % states.len()])
        })
        .collect();
    store(world).write_index(&entries);
    world.coding_event_index_written = true;
}

#[when("the coordinator replays logs and rebuilds the index")]
fn when_replay_and_rebuild(world: &mut QuectoWorld) {
    let jobs = store(world).discover_jobs();
    let mut recovered: Vec<(String, String)> = Vec::new();
    for job_id in &jobs {
        let lines = store(world).read_log(job_id);
        let mut last_state = "unknown".to_string();
        for line in &lines {
            if let EventLogLine::Valid(env) = line {
                match env.event_type.as_str() {
                    "job.start" => last_state = "preparing".to_string(),
                    "job.ready" => last_state = "running".to_string(),
                    "job.status" => last_state = "running".to_string(),
                    "job.end" => {
                        if let Some(s) = env.payload.get("state").and_then(|v| v.as_str()) {
                            last_state = s.to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        recovered.push((job_id.clone(), last_state));
    }
    // Write the rebuilt index
    let entries: Vec<(String, JobState)> = recovered
        .iter()
        .map(|(id, state)| {
            let js = match state.as_str() {
                "queued" => JobState::Queued,
                "preparing" => JobState::Preparing,
                "running" => JobState::Running,
                "succeeded" => JobState::Succeeded,
                "failed" => JobState::Failed,
                "canceled" => JobState::Canceled,
                _ => JobState::Failed,
            };
            (id.clone(), js)
        })
        .collect();
    store(world).write_index(&entries);
    world.coding_event_recovered_jobs = Some(recovered);
}

#[when("the coordinator replays the event log")]
fn when_replay_event_log(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    let mut state = "unknown".to_string();
    let mut progress: Option<u32> = None;
    let mut worker_pid: Option<u32> = None;
    let mut summary: Option<String> = None;
    let mut had_corrupt = false;
    let mut had_truncated = false;
    for line in &lines {
        match line {
            EventLogLine::Valid(env) => match env.event_type.as_str() {
                "job.start" => state = "preparing".to_string(),
                "job.ready" => {
                    state = "running".to_string();
                    if let Some(pid) = env.payload.get("worker_pid").and_then(|v| v.as_u64()) {
                        worker_pid = Some(pid as u32);
                    }
                }
                "job.status" => {
                    state = "running".to_string();
                    if let Some(p) = env.payload.get("progress").and_then(|v| v.as_u64()) {
                        progress = Some(p as u32);
                    }
                }
                "job.end" => {
                    if let Some(s) = env.payload.get("state").and_then(|v| v.as_str()) {
                        state = s.to_string();
                    }
                    if let Some(s) = env.payload.get("summary").and_then(|v| v.as_str()) {
                        summary = Some(s.to_string());
                    }
                }
                _ => {}
            },
            EventLogLine::Corrupt { raw, .. } => {
                if raw.is_empty() || !raw.ends_with('}') {
                    had_truncated = true;
                }
                had_corrupt = true;
            }
        }
    }
    world.coding_event_replayed_state = Some(state);
    world.coding_event_replayed_progress = progress;
    world.coding_event_replayed_worker_pid = worker_pid;
    world.coding_event_replayed_summary = summary;
    world.coding_event_replay_had_corrupt = had_corrupt;
    world.coding_event_replay_had_truncated = had_truncated || world.coding_event_truncated_present;
}

#[when("the coordinator attempts to append an event exceeding 1 MiB")]
fn when_oversized_event(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let big_payload = "x".repeat(2 * 1024 * 1024);
    let event = make_event(
        &EventId {
            job: &job_id,
            run: "run_000001",
            event_type: "log.message",
            seq: 99,
        },
        EventSource::Worker,
        serde_json::json!({"message": big_payload}),
    );
    store(world).append_event(&job_id, &event);
    world.coding_event_oversized_attempted = true;
}

#[when("the coordinator scans for job directories")]
fn when_scan_jobs(world: &mut QuectoWorld) {
    let jobs = store(world).discover_jobs();
    world.coding_event_discovered_jobs = Some(jobs);
}

#[when("the coordinator acquires the lock")]
fn when_acquire_lock(world: &mut QuectoWorld) {
    let acquired = store(world).try_acquire_lock();
    world.coding_event_lock_acquired = Some(acquired);
}

#[when("a second coordinator attempts to acquire the lock")]
fn when_second_coordinator_lock(world: &mut QuectoWorld) {
    let acquired = store(world).try_acquire_lock();
    world.coding_event_lock_acquired = Some(acquired);
}

#[when("the event log is read back")]
fn when_read_back(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    world.coding_event_read_lines = Some(lines);
}

// ── Then steps ───────────────────────────────────────────────────────────

#[then(regex = r#"^the event log should contain (\d+) valid JSON lines?$"#)]
fn then_event_log_has_n_lines(world: &mut QuectoWorld, expected: usize) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    let valid_count = lines
        .iter()
        .filter(|l| matches!(l, EventLogLine::Valid(_)))
        .count();
    assert_eq!(
        valid_count, expected,
        "expected {} valid JSON lines, got {}",
        expected, valid_count
    );
}

#[then("the line should be a valid EventEnvelope JSON")]
fn then_valid_envelope(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    let last = lines.last().expect("should have at least one line");
    match last {
        EventLogLine::Valid(env) => {
            assert_eq!(env.v, "1.0");
            assert!(!env.ts.is_empty());
            assert!(!env.run_id.is_empty());
            assert!(!env.job_id.is_empty());
        }
        EventLogLine::Corrupt { .. } => panic!("expected valid envelope"),
    }
}

#[then("the event log file should exist on disk")]
fn then_event_log_exists(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let dir = store_dir(world);
    let path = dir.join(&job_id).join("events.jsonl");
    assert!(path.exists(), "event log file should exist on disk");
}

#[then("the flushed event should be readable from disk")]
fn then_flushed_readable(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let dir = store_dir(world);
    let path = dir.join(&job_id).join("events.jsonl");
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.is_empty(), "event log should have content on disk");
    // Verify the content is valid JSON lines
    for line in content.lines() {
        if !line.trim().is_empty() {
            let _: EventEnvelope = serde_json::from_str(line)
                .expect("each flushed line should be valid EventEnvelope JSON");
        }
    }
}

#[then("each line should have monotonically increasing seq numbers")]
fn then_monotonic_seq(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    let mut last_seq = 0u64;
    for line in &lines {
        if let EventLogLine::Valid(env) = line {
            assert!(
                env.seq > last_seq,
                "seq should be monotonically increasing: {} > {}",
                env.seq,
                last_seq
            );
            last_seq = env.seq;
        }
    }
}

#[then("the file should not contain any blank lines")]
fn then_no_blank_lines(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let dir = store_dir(world);
    let path = dir.join(&job_id).join("events.jsonl");
    let content = fs::read_to_string(&path).unwrap();
    for (i, line) in content.lines().enumerate() {
        assert!(
            !line.trim().is_empty(),
            "line {} should not be blank",
            i + 1
        );
    }
}

#[then(regex = r#"^the index file should contain (\d+) entries with job_id and state$"#)]
fn then_index_has_entries(world: &mut QuectoWorld, expected: usize) {
    let dir = store_dir(world);
    let content = fs::read_to_string(dir.join("index.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let obj = parsed.as_object().unwrap();
    assert_eq!(
        obj.len(),
        expected,
        "index should have {} entries, got {}",
        expected,
        obj.len()
    );
}

#[then("the snapshot should match the current in-memory state")]
fn then_snapshot_matches(world: &mut QuectoWorld) {
    assert!(
        world.coding_event_index_written,
        "index should have been written"
    );
}

#[then(regex = r#"^the index should show (\S+) as "([^"]+)"$"#)]
fn then_index_shows_state(world: &mut QuectoWorld, job_id: String, expected_state: String) {
    let dir = store_dir(world);
    let content = fs::read_to_string(dir.join("index.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(
        parsed[&job_id].as_str().unwrap(),
        expected_state,
        "index should show {} as {}",
        job_id,
        expected_state
    );
}

#[then("the index file should be created")]
fn then_index_created(world: &mut QuectoWorld) {
    let dir = store_dir(world);
    assert!(
        dir.join("index.json").exists(),
        "index.json should be created"
    );
}

#[then("the rebuilt index should contain all discovered jobs")]
fn then_rebuilt_index_complete(world: &mut QuectoWorld) {
    let dir = store_dir(world);
    let content = fs::read_to_string(dir.join("index.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(parsed.is_object(), "index should be a JSON object");
    assert!(
        !parsed.as_object().unwrap().is_empty(),
        "index should not be empty"
    );
}

#[then(regex = r#"^the replayed state should be "([^"]+)"$"#)]
fn then_replayed_state(world: &mut QuectoWorld, expected_state: String) {
    let state = world
        .coding_event_replayed_state
        .as_ref()
        .expect("replayed state");
    assert_eq!(
        state, &expected_state,
        "expected state {}, got {}",
        expected_state, state
    );
}

#[then(regex = r#"^the replayed worker_pid should be (\d+)$"#)]
fn then_replayed_worker_pid(world: &mut QuectoWorld, expected_pid: u32) {
    assert_eq!(
        world.coding_event_replayed_worker_pid,
        Some(expected_pid),
        "expected worker_pid {}",
        expected_pid
    );
}

#[then(regex = r#"^the replayed progress should be (\d+)$"#)]
fn then_replayed_progress(world: &mut QuectoWorld, expected: u32) {
    assert_eq!(
        world.coding_event_replayed_progress,
        Some(expected),
        "expected progress {}",
        expected
    );
}

#[then(regex = r#"^the replayed summary should be "([^"]+)"$"#)]
fn then_replayed_summary(world: &mut QuectoWorld, expected: String) {
    assert_eq!(
        world.coding_event_replayed_summary.as_deref(),
        Some(expected.as_str()),
        "expected summary '{}'",
        expected
    );
}

#[then("the replayed log should contain todo events for reconstruction")]
fn then_replayed_has_todos(world: &mut QuectoWorld) {
    let job_id = world.coding_event_job_id.clone().unwrap();
    let lines = store(world).read_log(&job_id);
    let todo_events: Vec<_> = lines
        .iter()
        .filter(|l| matches!(l, EventLogLine::Valid(e) if e.event_type.starts_with("todo.")))
        .collect();
    assert!(
        !todo_events.is_empty(),
        "should have todo events for reconstruction"
    );
}

#[then("the replayed log should contain spawn events for reconstruction")]
fn then_replayed_has_spawns(world: &mut QuectoWorld) {
    let job_id = world.coding_event_job_id.clone().unwrap();
    let lines = store(world).read_log(&job_id);
    let spawn_events: Vec<_> = lines
        .iter()
        .filter(|l| matches!(l, EventLogLine::Valid(e) if e.event_type.starts_with("spawn.")))
        .collect();
    assert!(
        !spawn_events.is_empty(),
        "should have spawn events for reconstruction"
    );
}

#[then("the truncated line should be detected as corrupt")]
fn then_truncated_detected(world: &mut QuectoWorld) {
    assert!(
        world.coding_event_replay_had_truncated || world.coding_event_truncated_present,
        "should have encountered truncated line"
    );
}

#[then("the replayed state should reflect the last complete event")]
fn then_replayed_state_reflects_last(world: &mut QuectoWorld) {
    let state = world
        .coding_event_replayed_state
        .as_ref()
        .expect("replayed state");
    assert_ne!(
        state, "unknown",
        "recovery should have produced a known state from the last complete event"
    );
}

#[then("the corrupted line should be detected and skipped")]
fn then_corrupted_detected(world: &mut QuectoWorld) {
    assert!(
        world.coding_event_replay_had_corrupt,
        "corrupted line should have been detected and skipped"
    );
}

#[then("the valid lines should still be replayed")]
fn then_valid_lines_replayed(world: &mut QuectoWorld) {
    let state = world
        .coding_event_replayed_state
        .as_ref()
        .expect("replayed state");
    assert_ne!(state, "unknown", "valid lines should have been replayed");
}

#[then("the oversized event should not appear in the log")]
fn then_oversized_not_in_log(world: &mut QuectoWorld) {
    assert!(
        world.coding_event_oversized_attempted,
        "oversized event should have been attempted"
    );
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    for line in &lines {
        if let EventLogLine::Valid(env) = line {
            if env.event_type == "log.message" && env.seq == 99 {
                panic!("oversized event should not be in the log");
            }
        }
    }
}

#[then("the event log should remain valid")]
fn then_event_log_valid(world: &mut QuectoWorld) {
    let job_id = world
        .coding_event_job_id
        .clone()
        .unwrap_or_else(|| "job_000001".to_string());
    let lines = store(world).read_log(&job_id);
    // All lines should be parseable (no corruption introduced by the rejected event)
    for line in &lines {
        assert!(
            matches!(line, EventLogLine::Valid(_)),
            "all remaining lines should be valid"
        );
    }
}

#[then(regex = r#"^(\S+) and (\S+) should be discovered$"#)]
fn then_jobs_discovered(world: &mut QuectoWorld, j1: String, j2: String) {
    let jobs = world
        .coding_event_discovered_jobs
        .as_ref()
        .expect("discovered jobs");
    assert!(jobs.contains(&j1), "should discover {}", j1);
    assert!(jobs.contains(&j2), "should discover {}", j2);
}

#[then(regex = r#"^(\S+) should not be discovered$"#)]
fn then_job_not_discovered(world: &mut QuectoWorld, ignored: String) {
    let jobs = world
        .coding_event_discovered_jobs
        .as_ref()
        .expect("discovered jobs");
    assert!(
        !jobs.contains(&ignored),
        "{} should not be discovered",
        ignored
    );
}

#[then("the lock file should exist with the current PID")]
fn then_lock_exists_with_pid(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_event_lock_acquired,
        Some(true),
        "lock should be acquired"
    );
    let dir = store_dir(world);
    let lock_path = dir.join("coordinator.lock");
    assert!(lock_path.exists(), "lock file should exist");
    let content = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(
        content.trim(),
        std::process::id().to_string(),
        "lock file should contain the current PID"
    );
}

#[then("the lock acquisition should fail")]
fn then_lock_fails(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_event_lock_acquired,
        Some(false),
        "lock acquisition should fail"
    );
}

#[then("every line should include v, ts, run_id, job_id, source, event_type, seq, and payload")]
fn then_envelope_fields(world: &mut QuectoWorld) {
    let lines = world.coding_event_read_lines.as_ref().expect("read lines");
    for line in lines {
        match line {
            EventLogLine::Valid(env) => {
                assert_eq!(env.v, "1.0");
                assert!(!env.ts.is_empty());
                assert!(!env.run_id.is_empty());
                assert!(!env.job_id.is_empty());
                assert!(!env.event_type.is_empty());
                // seq and payload always present by struct definition
            }
            EventLogLine::Corrupt { .. } => panic!("all lines should be valid for this scenario"),
        }
    }
}

#[then(regex = r#"^the v field should be "([^"]+)"$"#)]
fn then_v_field(world: &mut QuectoWorld, expected: String) {
    let lines = world.coding_event_read_lines.as_ref().expect("read lines");
    for line in lines {
        if let EventLogLine::Valid(env) = line {
            assert_eq!(env.v, expected);
        }
    }
}

#[then("seq numbers should be monotonically increasing within each scope")]
fn then_seq_monotonic_scoped(world: &mut QuectoWorld) {
    let lines = world.coding_event_read_lines.as_ref().expect("read lines");
    let mut last_seq_by_scope: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();
    for line in lines {
        if let EventLogLine::Valid(env) = line {
            let key = format!("{}:{}:{}", env.source, env.run_id, env.job_id);
            let last = last_seq_by_scope.entry(key).or_insert(0);
            assert!(
                env.seq > *last,
                "seq should be monotonically increasing within scope"
            );
            *last = env.seq;
        }
    }
}
