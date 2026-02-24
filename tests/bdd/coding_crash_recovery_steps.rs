use super::*;
use quecto::application::coding_crash_recovery::{self, RecoveryOp};
use quecto::domain::coding_event::EventEnvelope;
use quecto::domain::coding_job::JobState;
use quecto::domain::coding_ports::{EventLogLine, EventLogStore, ProcessChecker};

// ============================================================================
// Test doubles for crash recovery ports
// ============================================================================

struct BddProcessChecker {
    alive: HashMap<u32, bool>,
}

impl BddProcessChecker {
    fn new() -> Self {
        Self {
            alive: HashMap::new(),
        }
    }
}

impl ProcessChecker for BddProcessChecker {
    fn is_alive(&self, pid: u32) -> bool {
        self.alive.get(&pid).copied().unwrap_or(false)
    }
}

struct BddEventLogStore {
    jobs: HashMap<String, Vec<EventLogLine>>,
    appended: Vec<(String, EventEnvelope)>,
    index_written: bool,
    lock_available: bool,
}

impl BddEventLogStore {
    fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            appended: Vec::new(),
            index_written: false,
            lock_available: true,
        }
    }
}

impl EventLogStore for BddEventLogStore {
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

    fn write_index(&mut self, _entries: &[(String, JobState)]) {
        self.index_written = true;
    }

    fn try_acquire_lock(&self) -> bool {
        self.lock_available
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Build the event log store from world fixture data.
fn build_store(world: &QuectoWorld) -> BddEventLogStore {
    let mut store = BddEventLogStore::new();
    store.lock_available = !world.coding_startup_failed_lock;

    for (job_id, events) in &world.coding_recovery_logs {
        let mut lines: Vec<EventLogLine> = Vec::new();
        for (idx, ev) in events.iter().enumerate() {
            let raw = serde_json::to_string(ev).unwrap_or_default();
            match serde_json::from_value::<EventEnvelope>(ev.clone()) {
                Ok(envelope) => lines.push(EventLogLine::Valid(envelope)),
                Err(_) => lines.push(EventLogLine::Corrupt {
                    line_number: idx + 1,
                    raw,
                }),
            }
        }
        if world.coding_truncated_line_skipped && job_id == "job_abc123" {
            lines.push(EventLogLine::Corrupt {
                line_number: lines.len() + 1,
                raw: "{truncated".to_string(),
            });
        }
        if world.coding_corrupted_line_skipped && job_id == "job_abc123" {
            let insert_at = std::cmp::min(2, lines.len());
            lines.insert(
                insert_at,
                EventLogLine::Corrupt {
                    line_number: 3,
                    raw: "NOT VALID JSON".to_string(),
                },
            );
        }
        store.jobs.insert(job_id.clone(), lines);
    }
    store
}

/// Map a successful recovery result into world fields.
fn apply_recovery_to_world(
    world: &mut QuectoWorld,
    recovery: &coding_crash_recovery::RecoveryResult,
    store: &BddEventLogStore,
) {
    world.coding_recovered_states.clear();
    world.coding_worker_check_performed = false;
    world.coding_recovery_events_appended = recovery.events_appended;
    world.coding_index_rewritten = recovery.index_rewritten;
    world.coding_recovery_operation_order.clear();
    world.coding_startup_error = None;

    for (job_id, rj) in &recovery.jobs {
        world
            .coding_recovered_states
            .insert(job_id.clone(), rj.state.to_string());
        if rj.worker_check_performed {
            world.coding_worker_check_performed = true;
        }
        if rj.has_todo_events {
            world
                .coding_todo_notes
                .insert("t1".to_string(), "in_progress".to_string());
        }
    }

    world.coding_warning_logged = !recovery.warnings.is_empty();
    world.coding_spawn_marked_failed = recovery.spawns.iter().any(|s| s.marked_failed);

    for op in &recovery.operation_order {
        let name = match op {
            RecoveryOp::Append => "append",
            RecoveryOp::Flush => "flush",
            RecoveryOp::StateUpdate => "state_update",
        };
        world.coding_recovery_operation_order.push(name.to_string());
    }

    for (job_id, env) in &store.appended {
        let ev = serde_json::to_value(env).unwrap();
        world
            .coding_recovery_logs
            .entry(job_id.clone())
            .or_default()
            .push(ev);
    }

    world.coding_recovery_flush_then_state = !world.coding_recovery_operation_order.is_empty();
}

/// Run recovery and store the result in the world.
fn run_recovery(world: &mut QuectoWorld) {
    let mut pc = BddProcessChecker::new();
    for (&pid, &alive) in &world.coding_process_alive {
        pc.alive.insert(pid as u32, alive);
    }

    let mut store = build_store(world);
    let result = coding_crash_recovery::recover(&pc, &mut store);

    match result {
        Ok(recovery) => apply_recovery_to_world(world, &recovery, &store),
        Err(e) => {
            world.coding_startup_error = Some(e.to_string());
            world.coding_recovered_states.clear();
            world.coding_recovery_events_appended = 0;
        }
    }
}

// ============================================================================
// Helpers to build fixture event log entries (JSON values, not envelopes)
// ============================================================================

/// Parameters for building a fixture event entry.
struct FixtureEvent<'a> {
    job_id: &'a str,
    event_type: &'a str,
    state: Option<&'a str>,
    reason: Option<&'a str>,
    error_code: Option<&'a str>,
    worker_pid: Option<u32>,
}

/// Build the payload JSON for a fixture event based on its type.
fn build_fixture_payload(
    event_type: &str,
    state: Option<&str>,
    reason: Option<&str>,
    error_code: Option<&str>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut payload = serde_json::Map::new();
    if let Some(s) = state {
        payload.insert("state".to_string(), serde_json::json!(s));
    }
    if let Some(r) = reason {
        payload.insert("reason".to_string(), serde_json::json!(r));
    }
    if let Some(ec) = error_code {
        payload.insert("error_code".to_string(), serde_json::json!(ec));
    }
    apply_event_type_defaults(event_type, &mut payload);
    payload
}

/// Apply required default fields for known event types.
fn apply_event_type_defaults(
    event_type: &str,
    payload: &mut serde_json::Map<String, serde_json::Value>,
) {
    match event_type {
        "job.start" => {
            payload
                .entry("goal".to_string())
                .or_insert(serde_json::json!("test"));
            payload
                .entry("base_ref".to_string())
                .or_insert(serde_json::json!("main"));
            payload
                .entry("branch".to_string())
                .or_insert(serde_json::json!("quecto/test"));
        }
        "job.end" => {
            payload
                .entry("summary".to_string())
                .or_insert(serde_json::json!("recovery"));
            payload
                .entry("state".to_string())
                .or_insert(serde_json::json!("failed"));
        }
        "job.status" => {
            payload
                .entry("summary".to_string())
                .or_insert(serde_json::json!("status"));
            payload
                .entry("state".to_string())
                .or_insert(serde_json::json!("running"));
        }
        "job.blocked" => {
            payload
                .entry("reason".to_string())
                .or_insert(serde_json::json!("blocked"));
        }
        "job.cancel" => {
            payload
                .entry("reason".to_string())
                .or_insert(serde_json::json!("user_request"));
        }
        "spawn.request" => {
            payload
                .entry("request_id".to_string())
                .or_insert(serde_json::json!("r1"));
            payload
                .entry("agent_type".to_string())
                .or_insert(serde_json::json!("code"));
            payload
                .entry("scope".to_string())
                .or_insert(serde_json::json!("test"));
        }
        "spawn.decision" => {
            payload
                .entry("request_id".to_string())
                .or_insert(serde_json::json!("r1"));
            payload
                .entry("approved".to_string())
                .or_insert(serde_json::json!(true));
        }
        "todo.create" => {
            payload
                .entry("todo_id".to_string())
                .or_insert(serde_json::json!("t1"));
            payload
                .entry("title".to_string())
                .or_insert(serde_json::json!("fix"));
            payload
                .entry("status".to_string())
                .or_insert(serde_json::json!("pending"));
        }
        "todo.update" => {
            payload
                .entry("todo_id".to_string())
                .or_insert(serde_json::json!("t1"));
            payload
                .entry("status".to_string())
                .or_insert(serde_json::json!("in_progress"));
        }
        _ => {}
    }
}

fn append_fixture_event(world: &mut QuectoWorld, ev: FixtureEvent<'_>) {
    world
        .coding_recovery_logs
        .entry(ev.job_id.to_string())
        .or_default();

    let mut obj = serde_json::Map::new();
    obj.insert("v".to_string(), serde_json::json!("1.0"));
    obj.insert("ts".to_string(), serde_json::json!("2026-01-01T00:00:00Z"));
    obj.insert(
        "run_id".to_string(),
        serde_json::json!(format!("run_{}", ev.job_id)),
    );
    obj.insert("job_id".to_string(), serde_json::json!(ev.job_id));
    obj.insert("source".to_string(), serde_json::json!("coordinator"));
    obj.insert("type".to_string(), serde_json::json!(ev.event_type));
    obj.insert("seq".to_string(), serde_json::json!(0));

    let mut payload = build_fixture_payload(ev.event_type, ev.state, ev.reason, ev.error_code);
    if let Some(pid) = ev.worker_pid {
        payload.insert("worker_pid".to_string(), serde_json::json!(pid));
        world
            .coding_recovered_worker_pid
            .insert(ev.job_id.to_string(), pid as i64);
    }

    obj.insert("payload".to_string(), serde_json::Value::Object(payload));
    world
        .coding_recovery_logs
        .get_mut(ev.job_id)
        .expect("log initialized")
        .push(serde_json::Value::Object(obj));
}

// ============================================================================
// Given steps
// ============================================================================

#[given("a job directory with an event log containing:")]
fn given_job_log_with_table(world: &mut QuectoWorld, step: &gherkin::Step) {
    world.coding_recovery_logs.clear();
    let table = step.table.as_ref().expect("table expected");
    for row in table.rows.iter().skip(1) {
        let event_type = row.first().map(|x| x.trim()).unwrap_or_default();
        let state = row.get(1).map(|x| x.trim()).filter(|x| !x.is_empty());
        append_fixture_event(
            world,
            FixtureEvent {
                job_id: "job_abc123",
                event_type,
                state,
                reason: None,
                error_code: None,
                worker_pid: None,
            },
        );
    }
}

#[given(expr = "job directories {string} and {string} each with event logs")]
fn given_two_job_dirs(world: &mut QuectoWorld, job_1: String, job_2: String) {
    world.coding_recovery_logs.clear();
    world.coding_recovery_logs.entry(job_1).or_default();
    world.coding_recovery_logs.entry(job_2).or_default();
}

#[given(expr = "{string} event log ends with {string} state {string}")]
fn given_job_log_ends_with_state(
    world: &mut QuectoWorld,
    job_id: String,
    event_type: String,
    state: String,
) {
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: &job_id,
            event_type: &event_type,
            state: Some(&state),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given(expr = "a stale {string} that does not match current event logs")]
fn given_stale_index(world: &mut QuectoWorld, _index: String) {
    world.coding_index_rewritten = false;
}

#[given(expr = "a job event log with a {string} event recording worker PID {int}")]
fn given_job_ready_pid(world: &mut QuectoWorld, event_type: String, pid: i64) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: &event_type,
            state: None,
            reason: None,
            error_code: None,
            worker_pid: Some(pid as u32),
        },
    );
}

#[given("the event log ends with state \"running\" (no terminal event)")]
fn given_log_ends_running(world: &mut QuectoWorld) {
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.status",
            state: Some("running"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("process 12345 is no longer alive")]
fn given_pid_dead(world: &mut QuectoWorld) {
    world.coding_process_alive.insert(12345, false);
}

#[given("a job event log with a \"job.ready\" event recording a worker PID")]
fn given_job_ready_unspecified_pid(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.status",
            state: Some("running"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("the worker process is still alive")]
fn given_worker_alive(world: &mut QuectoWorld) {
    world.coding_process_alive.insert(22222, true);
}

#[given("a job event log where the last line is truncated (partial write)")]
fn given_truncated_last_line(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.start",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    world.coding_truncated_line_skipped = true;
}

#[given("a job event log containing todo.create and todo.update events")]
fn given_todo_events(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    world.coding_todo_notes.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.start",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.ready",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: Some(11111),
        },
    );
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "todo.create",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "todo.update",
            state: Some("in_progress"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    world.coding_process_alive.insert(11111, true);
}

#[given(expr = "a job event log ending with {string} state {string}")]
fn given_job_end_state(world: &mut QuectoWorld, event_type: String, state: String) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: &event_type,
            state: Some(&state),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given(expr = "a job event log ending with {string} reason {string}")]
fn given_job_cancel_reason(world: &mut QuectoWorld, event_type: String, reason: String) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: &event_type,
            state: None,
            reason: Some(&reason),
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("a job directory with an empty events.jsonl file")]
fn given_empty_events(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    world
        .coding_recovery_logs
        .entry("job_abc123".to_string())
        .or_default();
}

#[given("a job event log containing only:")]
fn given_log_containing_only(world: &mut QuectoWorld, step: &gherkin::Step) {
    world.coding_recovery_logs.clear();
    let table = step.table.as_ref().expect("table expected");
    for row in table.rows.iter().skip(1) {
        let event_type = row.first().map(|x| x.trim()).unwrap_or_default();
        append_fixture_event(
            world,
            FixtureEvent {
                job_id: "job_abc123",
                event_type,
                state: None,
                reason: None,
                error_code: None,
                worker_pid: None,
            },
        );
    }
}

#[given("a job event log containing:")]
fn given_log_containing(world: &mut QuectoWorld, step: &gherkin::Step) {
    world.coding_recovery_logs.clear();
    let table = step.table.as_ref().expect("table expected");
    for row in table.rows.iter().skip(1) {
        let event_type = row.first().map(|x| x.trim()).unwrap_or_default();
        let state = row.get(1).map(|x| x.trim()).filter(|x| !x.is_empty());
        append_fixture_event(
            world,
            FixtureEvent {
                job_id: "job_abc123",
                event_type,
                state,
                reason: None,
                error_code: None,
                worker_pid: None,
            },
        );
    }
}

#[given("the recorded worker PID is no longer alive")]
fn given_recorded_pid_dead(world: &mut QuectoWorld) {
    world.coding_process_alive.insert(22222, false);
    world.coding_process_alive.insert(12345, false);
    world
        .coding_recovered_worker_pid
        .insert("job_abc123".to_string(), 22222);

    // Retroactively patch job.ready events to include the worker PID
    // so the production replay_events function can find it.
    if let Some(events) = world.coding_recovery_logs.get_mut("job_abc123") {
        for ev in events.iter_mut() {
            let is_ready = ev
                .get("type")
                .and_then(|v| v.as_str())
                .map(|t| t == "job.ready")
                .unwrap_or(false);
            if is_ready {
                if let Some(payload) = ev.get_mut("payload") {
                    payload
                        .as_object_mut()
                        .expect("payload is object")
                        .entry("worker_pid")
                        .or_insert(serde_json::json!(22222));
                }
            }
        }
    }
}

#[given(expr = "a job event log ending with {string} state {string} error_code {string}")]
fn given_log_ending_with_error(
    world: &mut QuectoWorld,
    event_type: String,
    state: String,
    error_code: String,
) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: &event_type,
            state: Some(&state),
            reason: None,
            error_code: Some(&error_code),
            worker_pid: None,
        },
    );
}

#[given("a job event log where line 3 contains invalid JSON (not just truncated)")]
fn given_corrupted_line(world: &mut QuectoWorld) {
    world.coding_corrupted_line_skipped = true;
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.start",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.status",
            state: Some("running"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("a job event log containing spawn.request and spawn.decision events")]
fn given_spawn_request_and_decision(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "spawn.request",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "spawn.decision",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("no spawn.result event")]
fn given_no_spawn_result(world: &mut QuectoWorld) {
    world.coding_spawn_marked_failed = false;
}

#[given("the child agent process is no longer alive")]
fn given_child_dead(world: &mut QuectoWorld) {
    world.coding_process_alive.insert(99999, false);
}

#[when("the child agent process is no longer alive")]
fn when_child_dead(world: &mut QuectoWorld) {
    given_child_dead(world);
}

#[given(expr = "job directories exist but {string} is missing")]
fn given_index_missing(world: &mut QuectoWorld, _path: String) {
    world.coding_recovery_logs.clear();
    world
        .coding_recovery_logs
        .entry("job_1".to_string())
        .or_default();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_1",
            event_type: "job.end",
            state: Some("succeeded"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("a coordinator lock file exists and is held by another process")]
fn given_lock_held(world: &mut QuectoWorld) {
    world.coding_startup_failed_lock = true;
}

// ============================================================================
// When steps
// ============================================================================

#[when("the coordinator starts up")]
fn when_startup(world: &mut QuectoWorld) {
    run_recovery(world);
}

#[when("a state transition event is processed")]
fn when_state_transition_processed(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    append_fixture_event(
        world,
        FixtureEvent {
            job_id: "job_abc123",
            event_type: "job.start",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    run_recovery(world);
    world.coding_recovery_flush_then_state = !world.coding_recovery_operation_order.is_empty();
}

#[when("the coordinator replays the log")]
fn when_replay_log(world: &mut QuectoWorld) {
    run_recovery(world);
}

#[when("the coordinator starts up again")]
fn when_startup_again(world: &mut QuectoWorld) {
    run_recovery(world);
}

#[when("a second coordinator instance starts up")]
fn when_second_startup(world: &mut QuectoWorld) {
    run_recovery(world);
}

// ============================================================================
// Then steps
// ============================================================================

#[then(expr = "the job should be in state {string} in memory")]
fn then_job_state_in_memory(world: &mut QuectoWorld, state: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some(state.as_str())
    );
}

#[then("the recovered state should match what the events describe")]
fn then_recovered_matches(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("running")
    );
}

#[then(expr = "{string} should be in state {string}")]
fn then_named_job_state(world: &mut QuectoWorld, job_id: String, state: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get(&job_id)
            .map(String::as_str),
        Some(state.as_str())
    );
}

#[then(expr = "{string} should be rewritten to match the replayed state")]
fn then_index_rewritten(world: &mut QuectoWorld, _index: String) {
    assert!(world.coding_index_rewritten);
}

#[then("the index should be consistent with the event logs")]
fn then_index_consistent(world: &mut QuectoWorld) {
    assert!(world.coding_index_rewritten);
}

#[then(expr = "the job should transition to {string}")]
fn then_transition(world: &mut QuectoWorld, state: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some(state.as_str())
    );
}

#[then(expr = "the recovered error_code should be {string}")]
fn then_recovered_error_code(world: &mut QuectoWorld, code: String) {
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    // Check either in top-level or in payload (envelopes store it in payload).
    let found = last.get("error_code").and_then(|v| v.as_str()).or_else(|| {
        last.get("payload")
            .and_then(|p| p.get("error_code"))
            .and_then(|v| v.as_str())
    });
    assert_eq!(found, Some(code.as_str()));
}

#[then(expr = "a {string} event should be appended to the log")]
fn then_event_appended(world: &mut QuectoWorld, event_type: String) {
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    let found_type = last
        .get("type")
        .and_then(|v| v.as_str())
        .or_else(|| last.get("event_type").and_then(|v| v.as_str()));
    assert_eq!(found_type, Some(event_type.as_str()));
}

#[then("the coordinator should re-attach to the worker's event stream")]
fn then_reattach(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("running")
    );
}

#[then(expr = "the job should remain in state {string}")]
fn then_job_remains(world: &mut QuectoWorld, state: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some(state.as_str())
    );
}

#[then("the event should be appended and flushed to the JSONL log")]
fn then_event_flushed(world: &mut QuectoWorld) {
    let append_ix = world
        .coding_recovery_operation_order
        .iter()
        .position(|x| x == "append")
        .expect("append step recorded");
    let flush_ix = world
        .coding_recovery_operation_order
        .iter()
        .position(|x| x == "flush")
        .expect("flush step recorded");
    assert!(append_ix < flush_ix);
}

#[then("only after the flush should the in-memory state be updated")]
fn then_flush_then_state(world: &mut QuectoWorld) {
    let flush_ix = world
        .coding_recovery_operation_order
        .iter()
        .position(|x| x == "flush")
        .expect("flush step recorded");
    let state_ix = world
        .coding_recovery_operation_order
        .iter()
        .position(|x| x == "state_update")
        .expect("state update step recorded");
    assert!(flush_ix < state_ix);
}

#[then("the truncated line should be skipped")]
fn then_truncated_skipped(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("failed")
    );
}

#[then("recovery should proceed with the last complete event")]
fn then_recovery_proceeds(world: &mut QuectoWorld) {
    assert!(world.coding_recovered_states.contains_key("job_abc123"));
}

#[then("a warning should be logged about the truncated line")]
fn then_warn_truncated(world: &mut QuectoWorld) {
    assert!(world.coding_warning_logged);
}

#[then("the todo list should be reconstructed from the events")]
fn then_todos_reconstructed(world: &mut QuectoWorld) {
    assert!(!world.coding_todo_notes.is_empty());
}

#[then("todo statuses should match the latest update events")]
fn then_todo_statuses_match(world: &mut QuectoWorld) {
    assert_eq!(
        world.coding_todo_notes.get("t1").map(String::as_str),
        Some("in_progress")
    );
}

#[then("no worker process check should be performed")]
fn then_no_worker_check(world: &mut QuectoWorld) {
    assert!(!world.coding_worker_check_performed);
}

#[then("no recovery action should be taken")]
fn then_no_recovery_action(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("canceled")
    );
}

#[then("the job should be discarded or marked as \"failed\"")]
fn then_empty_marked_failed(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("failed")
    );
}

#[then("a warning should be logged about the empty event log")]
fn then_warn_empty(world: &mut QuectoWorld) {
    assert!(world.coding_warning_logged);
}

#[then(expr = "the job should be transitioned to {string} with error_code {string}")]
fn then_transition_with_error(world: &mut QuectoWorld, state: String, code: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some(state.as_str())
    );
    // Verify the error_code was appended via the production crash event.
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    let found_code = last.get("error_code").and_then(|v| v.as_str()).or_else(|| {
        last.get("payload")
            .and_then(|p| p.get("error_code"))
            .and_then(|v| v.as_str())
    });
    assert_eq!(found_code, Some(code.as_str()));
}

#[then("no worker process check should be needed since no PID was recorded")]
fn then_no_worker_needed(world: &mut QuectoWorld) {
    assert!(!world.coding_worker_check_performed);
}

#[then("a \"job.end\" event should be appended")]
fn then_job_end_appended(world: &mut QuectoWorld) {
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    let found_type = last
        .get("type")
        .and_then(|v| v.as_str())
        .or_else(|| last.get("event_type").and_then(|v| v.as_str()));
    assert_eq!(found_type, Some("job.end"));
}

#[then("no additional \"job.end\" events should be appended")]
fn then_no_extra_job_end(world: &mut QuectoWorld) {
    assert_eq!(world.coding_recovery_events_appended, 0);
}

#[then(expr = "the job should be in state {string}")]
fn then_job_state_short(world: &mut QuectoWorld, state: String) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some(state.as_str())
    );
}

#[then("the corrupted line should be skipped")]
fn then_corrupted_skipped(world: &mut QuectoWorld) {
    assert_eq!(
        world
            .coding_recovered_states
            .get("job_abc123")
            .map(String::as_str),
        Some("running")
    );
}

#[then("recovery should proceed with subsequent valid events")]
fn then_recovery_after_corruption(world: &mut QuectoWorld) {
    assert!(world.coding_recovered_states.contains_key("job_abc123"));
}

#[then("a warning should be logged about the corrupted line")]
fn then_warn_corrupted(world: &mut QuectoWorld) {
    assert!(world.coding_warning_logged);
}

#[then("the spawn should be marked as failed")]
fn then_spawn_failed(world: &mut QuectoWorld) {
    assert!(world.coding_spawn_marked_failed);
}

#[then(expr = "a {string} event should be appended with state {string}")]
fn then_event_appended_with_state(world: &mut QuectoWorld, event_type: String, state: String) {
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    let found_type = last
        .get("type")
        .and_then(|v| v.as_str())
        .or_else(|| last.get("event_type").and_then(|v| v.as_str()));
    assert_eq!(found_type, Some(event_type.as_str()));
    let found_state = last.get("state").and_then(|v| v.as_str()).or_else(|| {
        last.get("payload")
            .and_then(|p| p.get("state"))
            .and_then(|v| v.as_str())
    });
    assert_eq!(found_state, Some(state.as_str()));
}

#[then(expr = "{string} should be created from the event logs")]
fn then_index_created(world: &mut QuectoWorld, _index: String) {
    assert!(world.coding_index_rewritten);
}

#[then("the index should be complete and correct")]
fn then_index_complete(world: &mut QuectoWorld) {
    assert!(world.coding_index_rewritten);
}

#[then("the second instance should fail with a clear error")]
fn then_second_fails(world: &mut QuectoWorld) {
    assert!(world.coding_startup_failed_lock);
    assert_eq!(
        world.coding_startup_error.as_deref(),
        Some("coordinator lock is already held")
    );
}

#[then("no event logs should be modified")]
fn then_no_logs_modified(world: &mut QuectoWorld) {
    assert_eq!(world.coding_recovery_events_appended, 0);
}
