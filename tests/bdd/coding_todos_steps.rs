use super::*;

use quecto::application::coding_todos::{
    TodoBlockedParams, TodoCompleteParams, TodoCreateParams, TodoError, TodoUpdateParams,
};
use quecto::domain::coding_command::TodoItem;

fn parse_list_literal(s: &str) -> Vec<String> {
    s.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn parse_kv_table(step: &gherkin::Step) -> serde_json::Map<String, serde_json::Value> {
    let table = step.table.as_ref().expect("expected table in step");
    table
        .rows
        .iter()
        .filter(|row| row.len() >= 2)
        .map(|row| {
            (
                row[0].trim().to_string(),
                serde_json::Value::String(row[1].trim().to_string()),
            )
        })
        .collect()
}

/// Return the current job_id from the coordinator world state.
fn current_job_id(world: &QuectoWorld) -> String {
    world
        .coding_current_job_id
        .clone()
        .expect("no current job_id — did Background run?")
}

/// Look up a todo item from the coordinator's tracker.
fn find_todo_in_tracker(world: &QuectoWorld, todo_id: &str) -> TodoItem {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    coord
        .todo_tracker()
        .todos_for_job(&jid)
        .iter()
        .find(|t| t.todo_id == todo_id)
        .cloned()
        .expect("todo not found in tracker")
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given(expr = "the job has todo {string} with status {string}")]
fn given_job_has_todo(world: &mut QuectoWorld, todo_id: String, status: String) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let tracker = coord.todo_tracker_mut();

    // Don't duplicate if already present (idempotent for Background reuse).
    if tracker
        .todos_for_job(&jid)
        .iter()
        .any(|t| t.todo_id == todo_id)
    {
        return;
    }

    tracker
        .create_todo(
            &jid,
            TodoCreateParams {
                todo_id: todo_id.clone(),
                title: "test todo".to_string(),
                owner: None,
                depends_on: vec![],
            },
        )
        .expect("create_todo in Given");

    // Advance through valid transitions to reach the target status.
    let to_ip = || TodoUpdateParams {
        todo_id: &todo_id,
        new_status: "in_progress",
        note: None,
    };
    match status.as_str() {
        "pending" => {}
        "in_progress" => {
            tracker
                .update_status(&jid, to_ip())
                .expect("transition to in_progress");
        }
        "completed" => {
            tracker
                .update_status(&jid, to_ip())
                .expect("transition to in_progress");
            tracker
                .complete_todo(
                    &jid,
                    TodoCompleteParams {
                        todo_id: &todo_id,
                        result: None,
                        artifact_refs: vec![],
                    },
                )
                .expect("complete_todo in Given");
        }
        "blocked" => {
            tracker
                .update_status(&jid, to_ip())
                .expect("transition to in_progress");
            tracker
                .block_todo(
                    &jid,
                    TodoBlockedParams {
                        todo_id: &todo_id,
                        reason: "setup".to_string(),
                        needs: None,
                    },
                )
                .expect("block_todo in Given");
        }
        "failed" => {
            tracker
                .update_status(&jid, to_ip())
                .expect("transition to in_progress");
            tracker
                .update_status(
                    &jid,
                    TodoUpdateParams {
                        todo_id: &todo_id,
                        new_status: "failed",
                        note: None,
                    },
                )
                .expect("transition to failed");
        }
        other => panic!("unsupported Given status: {other}"),
    }
}

#[given("the job has todos:")]
fn given_job_has_todos(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("table expected");
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let tracker = coord.todo_tracker_mut();

    for row in table.rows.iter().skip(1) {
        if row.len() < 3 {
            continue;
        }
        let todo_id = row[0].trim().to_string();
        let title = row[1].trim().to_string();
        let status = row[2].trim().to_string();
        let depends_on = if row.len() >= 4 {
            parse_list_literal(row[3].trim())
        } else {
            vec![]
        };

        tracker
            .create_todo(
                &jid,
                TodoCreateParams {
                    todo_id: todo_id.clone(),
                    title,
                    owner: None,
                    depends_on,
                },
            )
            .expect("create_todo in Given table");

        // Advance through valid transitions to reach the target status.
        let to_ip = || TodoUpdateParams {
            todo_id: &todo_id,
            new_status: "in_progress",
            note: None,
        };
        match status.as_str() {
            "pending" => {}
            "in_progress" => {
                tracker
                    .update_status(&jid, to_ip())
                    .expect("transition to in_progress");
            }
            "completed" => {
                tracker
                    .update_status(&jid, to_ip())
                    .expect("transition to in_progress");
                tracker
                    .complete_todo(
                        &jid,
                        TodoCompleteParams {
                            todo_id: &todo_id,
                            result: None,
                            artifact_refs: vec![],
                        },
                    )
                    .expect("complete_todo");
            }
            "blocked" => {
                tracker
                    .update_status(&jid, to_ip())
                    .expect("transition to in_progress");
                tracker
                    .block_todo(
                        &jid,
                        TodoBlockedParams {
                            todo_id: &todo_id,
                            reason: "setup".to_string(),
                            needs: None,
                        },
                    )
                    .expect("block_todo");
            }
            "failed" => {
                tracker
                    .update_status(&jid, to_ip())
                    .expect("transition to in_progress");
                tracker
                    .update_status(
                        &jid,
                        TodoUpdateParams {
                            todo_id: &todo_id,
                            new_status: "failed",
                            note: None,
                        },
                    )
                    .expect("transition to failed");
            }
            other => panic!("unsupported Given status: {other}"),
        }
    }
}

#[given(expr = "the coordinator is configured with max_items_per_job {int}")]
fn given_max_items(world: &mut QuectoWorld, max_items: usize) {
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    coord.todo_tracker_mut().set_max_items_per_job(max_items);
}

#[given(expr = "the job already has {int} todo items")]
fn given_job_already_has_n_todos(world: &mut QuectoWorld, count: usize) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let tracker = coord.todo_tracker_mut();
    for i in 0..count {
        tracker
            .create_todo(
                &jid,
                TodoCreateParams {
                    todo_id: format!("t{}", i + 1),
                    title: format!("Todo {}", i + 1),
                    owner: None,
                    depends_on: vec![],
                },
            )
            .expect("create_todo in Given N todos");
    }
}

// ── When steps ──────────────────────────────────────────────────────────

#[when(regex = r#"^the worker emits a \"(todo\.[^\"]+)\" event with:$"#)]
fn when_worker_emits_todo_event_with_table(
    world: &mut QuectoWorld,
    event_type: String,
    step: &gherkin::Step,
) {
    let fields = parse_kv_table(step);
    if event_type == "todo.create" {
        do_todo_create(world, &fields);
    }
}

#[when(expr = "the worker emits a {string} event for {string} with status {string}")]
fn when_worker_emits_update_status(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
    status: String,
) {
    if event_type == "todo.update" {
        do_todo_update(world, &todo_id, &status, None);
    }
}

#[when(expr = "the worker emits a {string} event for {string} with result {string}")]
fn when_worker_emits_complete_result(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
    result: String,
) {
    if event_type == "todo.complete" {
        do_todo_complete(world, &todo_id, Some(result), None);
    }
}

#[when(expr = "the worker emits a {string} event for {string} with artifact_refs {string}")]
fn when_worker_emits_complete_artifacts(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
    refs: String,
) {
    if event_type == "todo.complete" {
        do_todo_complete(world, &todo_id, None, Some(parse_list_literal(&refs)));
    }
}

#[when(
    regex = r#"^the worker emits a \"([^\"]+)\" event for \"([^\"]+)\" with artifact_refs (\[.*\])$"#
)]
fn when_worker_emits_complete_artifacts_unquoted(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
    refs: String,
) {
    when_worker_emits_complete_artifacts(world, event_type, todo_id, refs);
}

#[when(expr = "the worker emits a {string} event for {string} with reason {string}")]
fn when_worker_emits_blocked_reason(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
    reason: String,
) {
    if event_type == "todo.blocked" {
        do_todo_blocked(world, &todo_id, &reason, None);
    }
}

#[when(expr = "the worker emits a {string} event for {string} with:")]
fn when_worker_emits_todo_event_for_with_table(
    world: &mut QuectoWorld,
    event_type: String,
    _todo_id: String,
    step: &gherkin::Step,
) {
    let fields = parse_kv_table(step);
    if event_type == "todo.blocked" {
        let reason = fields
            .get("reason")
            .and_then(|v| v.as_str())
            .expect("reason required");
        let needs = fields
            .get("needs")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        do_todo_blocked(world, &_todo_id, reason, needs);
    } else if event_type == "todo.update" {
        let status = fields
            .get("status")
            .and_then(|v| v.as_str())
            .expect("status required");
        let note = fields
            .get("note")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        do_todo_update(world, &_todo_id, status, note);
    }
}

#[when(expr = "the worker emits a {string} event with todo_id {string}")]
fn when_worker_emits_create_duplicate(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
) {
    if event_type == "todo.create" {
        let fields = serde_json::Map::from_iter(vec![
            ("todo_id".to_string(), serde_json::Value::String(todo_id)),
            (
                "title".to_string(),
                serde_json::Value::String("duplicate".to_string()),
            ),
            (
                "status".to_string(),
                serde_json::Value::String("pending".to_string()),
            ),
        ]);
        do_todo_create(world, &fields);
    }
}

#[when("the worker emits a \"todo.create\" event for a 51st todo")]
fn when_worker_emits_51st_todo(world: &mut QuectoWorld) {
    let fields = serde_json::Map::from_iter(vec![
        (
            "todo_id".to_string(),
            serde_json::Value::String("t51".to_string()),
        ),
        (
            "title".to_string(),
            serde_json::Value::String("Todo 51".to_string()),
        ),
        (
            "status".to_string(),
            serde_json::Value::String("pending".to_string()),
        ),
    ]);
    do_todo_create(world, &fields);
}

#[when("the worker creates and completes a todo")]
fn when_worker_creates_and_completes_todo(world: &mut QuectoWorld) {
    let create_fields = serde_json::Map::from_iter(vec![
        (
            "todo_id".to_string(),
            serde_json::Value::String("t1".to_string()),
        ),
        (
            "title".to_string(),
            serde_json::Value::String("Create+complete".to_string()),
        ),
        (
            "status".to_string(),
            serde_json::Value::String("pending".to_string()),
        ),
    ]);
    do_todo_create(world, &create_fields);
    do_todo_update(world, "t1", "in_progress", None);
    do_todo_complete(world, "t1", Some("done".to_string()), None);
}

#[when("the parent job is canceled")]
fn when_parent_job_canceled(world: &mut QuectoWorld) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    coord.cancel(&jid).expect("cancel job");
}

// ── Production-calling helper functions ─────────────────────────────────

fn do_todo_create(world: &mut QuectoWorld, fields: &serde_json::Map<String, serde_json::Value>) {
    world.coding_todo_create_rejected = false;
    let jid = current_job_id(world);

    let todo_id = fields
        .get("todo_id")
        .and_then(|v| v.as_str())
        .expect("todo_id required")
        .to_string();
    let title = fields
        .get("title")
        .and_then(|v| v.as_str())
        .expect("title required")
        .to_string();
    let status = fields
        .get("status")
        .and_then(|v| v.as_str())
        .expect("status required");

    // Production code: todos always start as "pending".
    if status != "pending" {
        world.coding_todo_create_rejected = true;
        return;
    }

    let owner = fields
        .get("owner")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let depends_on = fields
        .get("depends_on")
        .and_then(|v| v.as_str())
        .map(parse_list_literal)
        .unwrap_or_default();

    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let result = coord.todo_tracker_mut().create_todo(
        &jid,
        TodoCreateParams {
            todo_id: todo_id.clone(),
            title: title.clone(),
            owner: owner.clone(),
            depends_on: depends_on.clone(),
        },
    );

    match result {
        Ok(()) => {
            // Emit event via coordinator for event-log scenarios.
            let mut payload =
                serde_json::json!({"todo_id": todo_id, "title": title, "status": "pending"});
            if let Some(o) = owner {
                payload["owner"] = serde_json::Value::String(o);
            }
            if !depends_on.is_empty() {
                payload["depends_on"] = serde_json::json!(depends_on);
            }
            coord
                .emit_worker_event(&jid, "todo.create", payload)
                .expect("emit_worker_event");
        }
        Err(TodoError::DuplicateId | TodoError::LimitReached) => {
            world.coding_todo_create_rejected = true;
        }
        Err(e) => panic!("unexpected todo create error: {e}"),
    }
}

fn do_todo_update(world: &mut QuectoWorld, todo_id: &str, status: &str, note: Option<String>) {
    world.coding_todo_transition_rejected = false;
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let result = coord.todo_tracker_mut().update_status(
        &jid,
        TodoUpdateParams {
            todo_id,
            new_status: status,
            note: note.clone(),
        },
    );

    match result {
        Ok(()) => {
            let mut payload = serde_json::json!({"todo_id": todo_id, "status": status});
            if let Some(n) = note {
                payload["note"] = serde_json::Value::String(n);
            }
            coord
                .emit_worker_event(&jid, "todo.update", payload)
                .expect("emit_worker_event");
        }
        Err(TodoError::InvalidTransition) => {
            world.coding_todo_transition_rejected = true;
        }
        Err(e) => panic!("unexpected todo update error: {e}"),
    }
}

fn do_todo_complete(
    world: &mut QuectoWorld,
    todo_id: &str,
    result: Option<String>,
    artifact_refs: Option<Vec<String>>,
) {
    world.coding_todo_transition_rejected = false;
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let outcome = coord.todo_tracker_mut().complete_todo(
        &jid,
        TodoCompleteParams {
            todo_id,
            result: result.clone(),
            artifact_refs: artifact_refs.clone().unwrap_or_default(),
        },
    );

    match outcome {
        Ok(()) => {
            let mut payload = serde_json::json!({"todo_id": todo_id, "result": ""});
            if let Some(res) = &result {
                payload["result"] = serde_json::Value::String(res.clone());
            }
            if let Some(refs) = &artifact_refs {
                payload["artifact_refs"] = serde_json::json!(refs);
            }
            coord
                .emit_worker_event(&jid, "todo.complete", payload)
                .expect("emit_worker_event");
        }
        Err(TodoError::InvalidTransition) => {
            world.coding_todo_transition_rejected = true;
        }
        Err(e) => panic!("unexpected todo complete error: {e}"),
    }
}

fn do_todo_blocked(world: &mut QuectoWorld, todo_id: &str, reason: &str, needs: Option<String>) {
    world.coding_todo_transition_rejected = false;
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_mut().expect("coordinator");
    let result = coord.todo_tracker_mut().block_todo(
        &jid,
        TodoBlockedParams {
            todo_id,
            reason: reason.to_string(),
            needs: needs.clone(),
        },
    );

    match result {
        Ok(()) => {
            let mut payload = serde_json::json!({"todo_id": todo_id, "reason": reason});
            if let Some(n) = needs {
                payload["needs"] = serde_json::Value::String(n);
            }
            coord
                .emit_worker_event(&jid, "todo.blocked", payload)
                .expect("emit_worker_event");
        }
        Err(TodoError::InvalidTransition) => {
            world.coding_todo_transition_rejected = true;
        }
        Err(e) => panic!("unexpected todo blocked error: {e}"),
    }
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then(expr = "the coordinator should record todo {string} with status {string}")]
fn then_recorded_todo_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then(expr = "the job's todo list should contain {int} item")]
fn then_todo_list_contains_one(world: &mut QuectoWorld, count: usize) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let todos = coord.todo_tracker().todos_for_job(&jid);
    assert_eq!(todos.len(), count);
}

#[then(expr = "todo {string} should have depends_on containing {string}")]
fn then_todo_depends_on(world: &mut QuectoWorld, todo_id: String, dep: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert!(
        todo.depends_on.contains(&dep),
        "todo {} depends_on {:?} does not contain {}",
        todo_id,
        todo.depends_on,
        dep
    );
}

#[then(expr = "todo {string} should have status {string}")]
fn then_todo_has_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then(expr = "the completion result should be {string}")]
fn then_completion_result(world: &mut QuectoWorld, result: String) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let tracker = coord.todo_tracker();
    let todos = tracker.todos_for_job(&jid);
    let found = todos
        .iter()
        .any(|t| tracker.todo_result(&jid, &t.todo_id) == Some(result.as_str()));
    assert!(found, "no todo has completion result '{result}'");
}

#[then(expr = "todo {string} should have artifact_refs containing {string}")]
fn then_todo_has_artifact_ref(world: &mut QuectoWorld, todo_id: String, artifact: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert!(
        todo.artifact_refs.contains(&artifact),
        "todo {} artifact_refs {:?} does not contain {}",
        todo_id,
        todo.artifact_refs,
        artifact
    );
}

#[then(expr = "the blocked reason should be {string}")]
fn then_blocked_reason(world: &mut QuectoWorld, reason: String) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let tracker = coord.todo_tracker();
    let todos = tracker.todos_for_job(&jid);
    let found = todos
        .iter()
        .any(|t| tracker.blocked_reason(&jid, &t.todo_id) == Some(reason.as_str()));
    assert!(found, "no todo has blocked reason '{reason}'");
}

#[then(expr = "the blocked event should include needs {string}")]
fn then_blocked_event_needs(world: &mut QuectoWorld, needs: String) {
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let event = coord
        .events()
        .iter()
        .rev()
        .find(|e| e.event_type == "todo.blocked")
        .expect("todo.blocked event not found");
    assert_eq!(event.payload["needs"], serde_json::Value::String(needs));
}

#[then(expr = "the status response should include {int} todo items")]
fn then_status_response_count(world: &mut QuectoWorld, count: usize) {
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response — did you call 'When the main agent queries job status'?")
        .todos;
    assert_eq!(todos.len(), count);
}

#[then("each todo should have todo_id, title, and status")]
fn then_each_todo_has_required_fields(world: &mut QuectoWorld) {
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos;
    assert!(!todos.is_empty());
    for todo in todos {
        assert!(!todo.todo_id.is_empty());
        assert!(!todo.title.is_empty());
        assert!(!todo.status.is_empty());
    }
}

#[then("the coordinator should reject the create with an error")]
fn then_reject_create(world: &mut QuectoWorld) {
    assert!(world.coding_todo_create_rejected);
}

#[then(expr = "the job should still have {int} todo items")]
fn then_job_still_has_todo_count(world: &mut QuectoWorld, count: usize) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let todos = coord.todo_tracker().todos_for_job(&jid);
    assert_eq!(todos.len(), count);
}

#[then(expr = "the job should still have {int} todo item")]
fn then_job_still_has_todo_count_singular(world: &mut QuectoWorld, count: usize) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let todos = coord.todo_tracker().todos_for_job(&jid);
    assert_eq!(todos.len(), count);
}

#[then("the event log should contain both \"todo.create\" and \"todo.complete\" events")]
fn then_event_log_contains_create_and_complete(world: &mut QuectoWorld) {
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let events = coord.events();
    assert!(
        events.iter().any(|e| e.event_type == "todo.create"),
        "no todo.create event found"
    );
    assert!(
        events.iter().any(|e| e.event_type == "todo.complete"),
        "no todo.complete event found"
    );
}

#[then("the events should have correct envelope fields")]
fn then_events_have_envelope_fields(world: &mut QuectoWorld) {
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let events = coord.events();
    let todo_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type.starts_with("todo."))
        .collect();
    assert!(!todo_events.is_empty(), "no todo events found");
    for e in todo_events {
        assert!(!e.v.is_empty());
        assert!(!e.ts.is_empty());
        assert!(!e.run_id.is_empty());
        assert!(!e.job_id.is_empty());
        assert!(!e.event_type.is_empty());
        assert!(e.seq >= 1);
        assert!(e.payload.is_object());
    }
}

#[then("all non-terminal todos should transition to \"canceled\"")]
fn then_non_terminal_todos_canceled(world: &mut QuectoWorld) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    let todos = coord.todo_tracker().todos_for_job(&jid);
    for todo in todos {
        assert!(
            matches!(todo.status.as_str(), "completed" | "failed" | "canceled"),
            "todo {} has status '{}' — expected terminal or canceled",
            todo.todo_id,
            todo.status
        );
    }
}

#[then(expr = "todo {string} should have owner {string}")]
fn then_todo_owner(world: &mut QuectoWorld, todo_id: String, owner: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert_eq!(todo.owner.as_deref(), Some(owner.as_str()));
}

#[then(expr = "todo {string} should have the note {string}")]
fn then_todo_note(world: &mut QuectoWorld, todo_id: String, note: String) {
    let jid = current_job_id(world);
    let coord = world.coding_coordinator.as_ref().expect("coordinator");
    assert_eq!(
        coord.todo_tracker().note(&jid, &todo_id),
        Some(note.as_str())
    );
}

#[then("the coordinator should reject the transition")]
fn then_transition_rejected(world: &mut QuectoWorld) {
    assert!(world.coding_todo_transition_rejected);
}

#[then(expr = "todo {string} should remain in status {string}")]
fn then_todo_remains_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo_in_tracker(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then("the coordinator should reject the create as duplicate")]
fn then_reject_duplicate_create(world: &mut QuectoWorld) {
    assert!(world.coding_todo_create_rejected);
}
