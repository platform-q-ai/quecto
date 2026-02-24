use super::*;

// -- Test doubles ---------------------------------------------------------

struct MockProcessChecker {
    alive: HashMap<u32, bool>,
}

impl MockProcessChecker {
    fn new() -> Self {
        Self {
            alive: HashMap::new(),
        }
    }
    fn set(&mut self, pid: u32, alive: bool) {
        self.alive.insert(pid, alive);
    }
}

impl ProcessChecker for MockProcessChecker {
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.get(&pid).copied().unwrap_or(false)
    }
}

struct MockEventLogStore {
    jobs: HashMap<String, Vec<EventLogLine>>,
    appended: Vec<(String, EventEnvelope)>,
    index_written: Vec<(String, JobState)>,
    lock_available: bool,
}

impl MockEventLogStore {
    fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            appended: Vec::new(),
            index_written: Vec::new(),
            lock_available: true,
        }
    }

    fn add_job(&mut self, job_id: &str, lines: Vec<EventLogLine>) {
        self.jobs.insert(job_id.to_string(), lines);
    }
}

impl EventLogStore for MockEventLogStore {
    fn discover_jobs(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.jobs.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn read_log(&self, job_id: &str) -> Vec<EventLogLine> {
        self.jobs.get(job_id).cloned().unwrap_or_default()
    }

    fn append_event(&mut self, job_id: &str, event: &EventEnvelope) {
        self.appended.push((job_id.to_string(), event.clone()));
    }

    fn write_index(&mut self, entries: &[(String, JobState)]) {
        self.index_written = entries.to_vec();
    }

    fn try_acquire_lock(&self) -> bool {
        self.lock_available
    }
}

fn envelope(job_id: &str, event_type: &str, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id: format!("run_{job_id}"),
        job_id: job_id.to_string(),
        source: EventSource::Coordinator,
        event_type: event_type.to_string(),
        seq: 0,
        payload,
    }
}

fn valid(env: EventEnvelope) -> EventLogLine {
    EventLogLine::Valid(env)
}

// -- Tests ----------------------------------------------------------------

#[test]
fn test_recover_single_running_job() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j1",
                "job.ready",
                serde_json::json!({"worker_pid": 1234}),
            )),
            valid(envelope(
                "j1",
                "job.status",
                serde_json::json!({"state": "running", "summary": "ok"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(1234, true);

    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Running);
    assert!(job.worker_check_performed);
    assert_eq!(result.events_appended, 0);
}

#[test]
fn test_recover_dead_worker_fails_job() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j1",
                "job.ready",
                serde_json::json!({"worker_pid": 5555}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(5555, false);

    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error_code.as_deref(), Some("coordinator_crash"));
    assert!(result.events_appended > 0);
}

#[test]
fn test_recover_terminal_job_no_worker_check() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.end",
            serde_json::json!({"state": "succeeded", "summary": "done"}),
        ))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    assert!(!job.worker_check_performed);
    assert_eq!(result.events_appended, 0);
}

#[test]
fn test_recover_canceled_job() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.cancel",
            serde_json::json!({"reason": "user_request"}),
        ))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Canceled);
    assert_eq!(result.events_appended, 0);
}

#[test]
fn test_recover_empty_log_fails_job() {
    let mut store = MockEventLogStore::new();
    store.add_job("j1", vec![]);
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(result.warnings.len(), 1);
    assert!(matches!(
        result.warnings[0],
        RecoveryWarning::EmptyEventLog { .. }
    ));
}

#[test]
fn test_recover_preparing_only_fails() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope("j1", "job.start", serde_json::json!({})))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error_code.as_deref(), Some("coordinator_crash"));
    assert!(!job.worker_check_performed);
}

#[test]
fn test_recover_corrupted_line_skipped() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            EventLogLine::Corrupt {
                line_number: 2,
                raw: "{bad json".to_string(),
            },
            valid(envelope(
                "j1",
                "job.status",
                serde_json::json!({"state": "running", "summary": "ok"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(0, true); // No worker PID recorded
    let result = recover(&pc, &mut store).unwrap();
    assert_eq!(result.warnings.len(), 1);
    assert!(matches!(
        result.warnings[0],
        RecoveryWarning::CorruptedLine { line_number: 2, .. }
    ));
}

#[test]
fn test_recover_multiple_jobs() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.end",
            serde_json::json!({"state": "succeeded", "summary": "ok"}),
        ))],
    );
    store.add_job(
        "j2",
        vec![
            valid(envelope("j2", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j2",
                "job.ready",
                serde_json::json!({"worker_pid": 9999}),
            )),
            valid(envelope(
                "j2",
                "job.status",
                serde_json::json!({"state": "running", "summary": "wip"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(9999, true);
    let result = recover(&pc, &mut store).unwrap();
    assert_eq!(result.jobs.get("j1").unwrap().state, JobState::Succeeded);
    assert_eq!(result.jobs.get("j2").unwrap().state, JobState::Running);
}

#[test]
fn test_recover_index_rewritten() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.end",
            serde_json::json!({"state": "failed", "summary": "err"}),
        ))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    assert!(result.index_rewritten);
    assert!(!store.index_written.is_empty());
}

#[test]
fn test_recover_lock_held_fails() {
    let mut store = MockEventLogStore::new();
    store.lock_available = false;
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().to_string(),
        "coordinator lock is already held"
    );
}

#[test]
fn test_recover_idempotent_double_crash() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.end",
            serde_json::json!({
                "state": "failed",
                "summary": "crash",
                "error_code": "coordinator_crash",
            }),
        ))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(result.events_appended, 0);
}

#[test]
fn test_recover_operation_order_durability() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope("j1", "job.start", serde_json::json!({})))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    // Must see append before flush before state_update
    let append_pos = result
        .operation_order
        .iter()
        .position(|o| *o == RecoveryOp::Append);
    let flush_pos = result
        .operation_order
        .iter()
        .position(|o| *o == RecoveryOp::Flush);
    let state_pos = result
        .operation_order
        .iter()
        .position(|o| *o == RecoveryOp::StateUpdate);
    assert!(append_pos.unwrap() < flush_pos.unwrap());
    assert!(flush_pos.unwrap() < state_pos.unwrap());
}

#[test]
fn test_recover_blocked_job_dead_worker() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j1",
                "job.ready",
                serde_json::json!({"worker_pid": 7777}),
            )),
            valid(envelope(
                "j1",
                "job.status",
                serde_json::json!({"state": "running", "summary": "wip"}),
            )),
            valid(envelope(
                "j1",
                "job.blocked",
                serde_json::json!({"reason": "need input"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(7777, false);
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error_code.as_deref(), Some("coordinator_crash"));
}

#[test]
fn test_recover_spawn_pending_dead_child() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope(
                "j1",
                "spawn.request",
                serde_json::json!({
                    "request_id": "r1",
                    "agent_type": "code",
                    "scope": "test",
                }),
            )),
            valid(envelope(
                "j1",
                "spawn.decision",
                serde_json::json!({"request_id": "r1", "approved": true}),
            )),
        ],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    assert_eq!(result.spawns.len(), 1);
    assert!(result.spawns[0].marked_failed);
    let spawn_events: Vec<_> = store
        .appended
        .iter()
        .filter(|(_, e)| e.event_type == "spawn.result")
        .collect();
    assert_eq!(spawn_events.len(), 1);
}

#[test]
fn test_recover_failed_job_no_worker_check() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![valid(envelope(
            "j1",
            "job.end",
            serde_json::json!({"state": "failed", "summary": "err"}),
        ))],
    );
    let pc = MockProcessChecker::new();
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert!(!job.worker_check_performed);
}

#[test]
fn test_recover_alive_worker_keeps_running() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j1",
                "job.ready",
                serde_json::json!({"worker_pid": 3333}),
            )),
            valid(envelope(
                "j1",
                "job.status",
                serde_json::json!({"state": "running", "summary": "ok"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(3333, true);
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Running);
    assert_eq!(result.events_appended, 0);
}

#[test]
fn test_recover_todo_events_tracked() {
    let mut store = MockEventLogStore::new();
    store.add_job(
        "j1",
        vec![
            valid(envelope("j1", "job.start", serde_json::json!({}))),
            valid(envelope(
                "j1",
                "job.ready",
                serde_json::json!({"worker_pid": 1111}),
            )),
            valid(envelope(
                "j1",
                "todo.create",
                serde_json::json!({
                    "todo_id": "t1",
                    "title": "fix",
                    "status": "pending",
                }),
            )),
            valid(envelope(
                "j1",
                "todo.update",
                serde_json::json!({"todo_id": "t1", "status": "in_progress"}),
            )),
        ],
    );
    let mut pc = MockProcessChecker::new();
    pc.set(1111, true);
    let result = recover(&pc, &mut store).unwrap();
    let job = result.jobs.get("j1").unwrap();
    assert_eq!(job.state, JobState::Running);
    assert!(job.has_todo_events);
}

#[test]
fn test_replay_events_returns_correct_state() {
    let events = vec![
        envelope("j1", "job.start", serde_json::json!({})),
        envelope("j1", "job.ready", serde_json::json!({"worker_pid": 42})),
        envelope(
            "j1",
            "job.status",
            serde_json::json!({"state": "running", "summary": "ok"}),
        ),
    ];
    let result = replay_events(&events);
    assert_eq!(result.state, JobState::Running);
    assert_eq!(result.worker_pid, Some(42));
    assert!(!result.terminal);
}

#[test]
fn test_replay_events_cancel_is_terminal() {
    let events = vec![envelope(
        "j1",
        "job.cancel",
        serde_json::json!({"reason": "user_request"}),
    )];
    let result = replay_events(&events);
    assert_eq!(result.state, JobState::Canceled);
    assert!(result.terminal);
}

#[test]
fn test_replay_events_spawn_tracking() {
    let events = vec![
        envelope(
            "j1",
            "spawn.request",
            serde_json::json!({
                "request_id": "r1",
                "agent_type": "x",
                "scope": "y",
            }),
        ),
        envelope(
            "j1",
            "spawn.decision",
            serde_json::json!({"request_id": "r1", "approved": true}),
        ),
    ];
    let result = replay_events(&events);
    assert!(result.has_pending_spawn);

    // Now add spawn.result
    let mut events2 = events;
    events2.push(envelope(
        "j1",
        "spawn.result",
        serde_json::json!({"request_id": "r1", "state": "succeeded"}),
    ));
    let result2 = replay_events(&events2);
    assert!(!result2.has_pending_spawn);
}

#[test]
fn test_make_crash_event_fields() {
    let ev = make_crash_event("j1", "r1");
    assert_eq!(ev.event_type, "job.end");
    assert_eq!(ev.job_id, "j1");
    assert_eq!(ev.run_id, "r1");
    assert_eq!(ev.payload["error_code"].as_str(), Some("coordinator_crash"));
    assert_eq!(ev.payload["state"].as_str(), Some("failed"));
}

#[test]
fn test_make_spawn_fail_event_fields() {
    let ev = make_spawn_fail_event("j1", "r1");
    assert_eq!(ev.event_type, "spawn.result");
    assert_eq!(ev.payload["state"].as_str(), Some("failed"));
}
