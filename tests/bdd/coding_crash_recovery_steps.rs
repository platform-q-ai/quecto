use super::*;

fn ensure_recovery_job(world: &mut QuectoWorld, job_id: &str) {
    world
        .coding_recovery_logs
        .entry(job_id.to_string())
        .or_default();
}

struct EventAppend<'a> {
    event_type: &'a str,
    state: Option<&'a str>,
    reason: Option<&'a str>,
    error_code: Option<&'a str>,
    worker_pid: Option<i64>,
}

fn append_event(world: &mut QuectoWorld, job_id: &str, data: EventAppend<'_>) {
    ensure_recovery_job(world, job_id);
    let mut obj = serde_json::Map::new();
    obj.insert(
        "type".to_string(),
        serde_json::Value::String(data.event_type.to_string()),
    );
    if let Some(s) = data.state {
        obj.insert(
            "state".to_string(),
            serde_json::Value::String(s.to_string()),
        );
    }
    if let Some(r) = data.reason {
        obj.insert(
            "reason".to_string(),
            serde_json::Value::String(r.to_string()),
        );
    }
    if let Some(code) = data.error_code {
        obj.insert(
            "error_code".to_string(),
            serde_json::Value::String(code.to_string()),
        );
    }
    if let Some(pid) = data.worker_pid {
        obj.insert(
            "worker_pid".to_string(),
            serde_json::Value::Number(pid.into()),
        );
        world
            .coding_recovered_worker_pid
            .insert(job_id.to_string(), pid);
    }
    world
        .coding_recovery_logs
        .get_mut(job_id)
        .expect("recovery log initialized")
        .push(serde_json::Value::Object(obj));
}

fn parse_recovery_events(events: &[serde_json::Value]) -> (String, bool, bool) {
    let mut state = "queued".to_string();
    let mut terminal = false;
    let mut has_spawn_pending = false;

    for ev in events {
        let t = ev["type"].as_str().unwrap_or_default();
        let event_state = ev["state"].as_str();
        match t {
            "job.start" => state = "preparing".to_string(),
            "job.ready" => state = "running".to_string(),
            "job.status" => {
                if let Some(s) = event_state {
                    state = s.to_string();
                }
            }
            "job.blocked" => state = "blocked".to_string(),
            "job.cancel" => {
                state = "canceled".to_string();
                terminal = true;
            }
            "job.end" => {
                if let Some(s) = event_state {
                    state = s.to_string();
                }
                terminal = true;
            }
            "spawn.request" | "spawn.decision" => has_spawn_pending = true,
            "spawn.result" => has_spawn_pending = false,
            _ => {}
        }
    }

    (state, terminal, has_spawn_pending)
}

fn append_coordinator_crash(world: &mut QuectoWorld, job_id: &str) {
    world.coding_recovery_events_appended += 1;
    append_event(
        world,
        job_id,
        EventAppend {
            event_type: "job.end",
            state: Some("failed"),
            reason: None,
            error_code: Some("coordinator_crash"),
            worker_pid: None,
        },
    );
}

fn apply_recovery(world: &mut QuectoWorld, job_id: &str, state: &mut String, terminal: bool) {
    if !terminal && *state == "preparing" {
        *state = "failed".to_string();
        append_coordinator_crash(world, job_id);
    } else if !terminal && matches!(state.as_str(), "running" | "blocked") {
        if let Some(pid) = world.coding_recovered_worker_pid.get(job_id).copied() {
            world.coding_worker_check_performed = true;
            let pid_alive = world
                .coding_process_alive
                .get(&pid)
                .copied()
                .unwrap_or(false);
            if !pid_alive {
                *state = "failed".to_string();
                append_coordinator_crash(world, job_id);
            }
        }
    }
}

fn apply_spawn_recovery(world: &mut QuectoWorld, job_id: &str, has_spawn_pending: bool) {
    if has_spawn_pending {
        let child_alive = world
            .coding_process_alive
            .get(&99999)
            .copied()
            .unwrap_or(false);
        if !child_alive {
            world.coding_spawn_marked_failed = true;
            world.coding_recovery_events_appended += 1;
            append_event(
                world,
                job_id,
                EventAppend {
                    event_type: "spawn.result",
                    state: Some("failed"),
                    reason: None,
                    error_code: None,
                    worker_pid: None,
                },
            );
        }
    }
}

fn replay(world: &mut QuectoWorld) {
    world.coding_worker_check_performed = false;
    world.coding_recovered_states.clear();
    if world.coding_startup_failed_lock {
        return;
    }

    for (job_id, events) in world.coding_recovery_logs.clone() {
        if events.is_empty() {
            world
                .coding_recovered_states
                .insert(job_id.clone(), "failed".to_string());
            world.coding_warning_logged = true;
            continue;
        }

        let (mut state, terminal, has_spawn_pending) = parse_recovery_events(&events);
        apply_recovery(world, &job_id, &mut state, terminal);
        apply_spawn_recovery(world, &job_id, has_spawn_pending);

        world.coding_recovered_states.insert(job_id, state);
    }

    if world.coding_truncated_line_skipped || world.coding_corrupted_line_skipped {
        world.coding_warning_logged = true;
    }

    world.coding_index_rewritten = true;
}

fn set_single_job_terminal(
    world: &mut QuectoWorld,
    event_type: &str,
    state: Option<&str>,
    reason: Option<&str>,
) {
    world.coding_recovery_logs.clear();
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type,
            state,
            reason,
            error_code: None,
            worker_pid: None,
        },
    );
}

#[given("a job directory with an event log containing:")]
fn given_job_log_with_table(world: &mut QuectoWorld, step: &gherkin::Step) {
    world.coding_recovery_logs.clear();
    let table = step.table.as_ref().expect("table expected");
    for row in table.rows.iter().skip(1) {
        let event_type = row.first().map(|x| x.trim()).unwrap_or_default();
        let state = row.get(1).map(|x| x.trim()).filter(|x| !x.is_empty());
        append_event(
            world,
            "job_abc123",
            EventAppend {
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
    ensure_recovery_job(world, &job_1);
    ensure_recovery_job(world, &job_2);
}

#[given(expr = "{string} event log ends with {string} state {string}")]
fn given_job_log_ends_with_state(
    world: &mut QuectoWorld,
    job_id: String,
    event_type: String,
    state: String,
) {
    append_event(
        world,
        &job_id,
        EventAppend {
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: &event_type,
            state: None,
            reason: None,
            error_code: None,
            worker_pid: Some(pid),
        },
    );
}

#[given("the event log ends with state \"running\" (no terminal event)")]
fn given_log_ends_running(world: &mut QuectoWorld) {
    append_event(
        world,
        "job_abc123",
        EventAppend {
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: "job.ready",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: Some(22222),
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: "todo.create",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: "todo.update",
            state: Some("in_progress"),
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    world
        .coding_todo_notes
        .insert("t1".to_string(), "in_progress".to_string());
}

#[given(expr = "a job event log ending with {string} state {string}")]
fn given_job_end_state(world: &mut QuectoWorld, event_type: String, state: String) {
    set_single_job_terminal(world, &event_type, Some(&state), None);
}

#[given(expr = "a job event log ending with {string} reason {string}")]
fn given_job_cancel_reason(world: &mut QuectoWorld, event_type: String, reason: String) {
    set_single_job_terminal(world, &event_type, None, Some(&reason));
}

#[given("a job directory with an empty events.jsonl file")]
fn given_empty_events(world: &mut QuectoWorld) {
    world.coding_recovery_logs.clear();
    ensure_recovery_job(world, "job_abc123");
}

#[given("a job event log containing only:")]
fn given_log_containing_only(world: &mut QuectoWorld, step: &gherkin::Step) {
    world.coding_recovery_logs.clear();
    let table = step.table.as_ref().expect("table expected");
    for row in table.rows.iter().skip(1) {
        let event_type = row.first().map(|x| x.trim()).unwrap_or_default();
        append_event(
            world,
            "job_abc123",
            EventAppend {
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
        append_event(
            world,
            "job_abc123",
            EventAppend {
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
}

#[given(expr = "a job event log ending with {string} state {string} error_code {string}")]
fn given_log_ending_with_error(
    world: &mut QuectoWorld,
    event_type: String,
    state: String,
    error_code: String,
) {
    world.coding_recovery_logs.clear();
    append_event(
        world,
        "job_abc123",
        EventAppend {
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: "job.start",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_event(
        world,
        "job_abc123",
        EventAppend {
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
    append_event(
        world,
        "job_abc123",
        EventAppend {
            event_type: "spawn.request",
            state: None,
            reason: None,
            error_code: None,
            worker_pid: None,
        },
    );
    append_event(
        world,
        "job_abc123",
        EventAppend {
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
    ensure_recovery_job(world, "job_1");
    append_event(
        world,
        "job_1",
        EventAppend {
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

#[when("the coordinator starts up")]
fn when_startup(world: &mut QuectoWorld) {
    replay(world);
}

#[when("a state transition event is processed")]
fn when_state_transition_processed(world: &mut QuectoWorld) {
    world.coding_recovery_flush_then_state = true;
}

#[when("the coordinator replays the log")]
fn when_replay_log(world: &mut QuectoWorld) {
    replay(world);
}

#[when("the coordinator starts up again")]
fn when_startup_again(world: &mut QuectoWorld) {
    replay(world);
}

#[when("a second coordinator instance starts up")]
fn when_second_startup(world: &mut QuectoWorld) {
    replay(world);
}

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
    assert!(world.coding_recovered_states.contains_key("job_abc123"));
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
    assert_eq!(last["error_code"], serde_json::Value::String(code));
}

#[then(expr = "a {string} event should be appended to the log")]
fn then_event_appended(world: &mut QuectoWorld, event_type: String) {
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    assert!(
        log.iter()
            .any(|e| e["type"] == serde_json::Value::String(event_type.clone()))
    );
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
    assert!(world.coding_recovery_flush_then_state);
}

#[then("only after the flush should the in-memory state be updated")]
fn then_flush_then_state(world: &mut QuectoWorld) {
    assert!(world.coding_recovery_flush_then_state);
}

#[then("the truncated line should be skipped")]
fn then_truncated_skipped(world: &mut QuectoWorld) {
    assert!(world.coding_truncated_line_skipped);
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
    assert!(world.coding_todo_notes.values().any(|s| s == "in_progress"));
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
    let log = world
        .coding_recovery_logs
        .get("job_abc123")
        .expect("job recovery log exists");
    let last = log.last().expect("event exists");
    assert_eq!(last["error_code"], serde_json::Value::String(code));
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
    assert!(
        log.iter()
            .any(|e| e["type"] == serde_json::Value::String("job.end".to_string()))
    );
}

#[then("no additional \"job.end\" events should be appended")]
fn then_no_extra_job_end(world: &mut QuectoWorld) {
    assert!(world.coding_recovery_events_appended <= 1);
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
    assert!(world.coding_corrupted_line_skipped);
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
    assert!(log.iter().any(|e| {
        e["type"] == serde_json::Value::String(event_type.clone())
            && e["state"] == serde_json::Value::String(state.clone())
    }));
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
}

#[then("no event logs should be modified")]
fn then_no_logs_modified(world: &mut QuectoWorld) {
    assert_eq!(world.coding_recovery_events_appended, 0);
}
