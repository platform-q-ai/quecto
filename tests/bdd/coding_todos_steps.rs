use super::*;

use quecto::domain::coding_command::{StatusResponse, TodoItem};
use quecto::domain::coding_event::{EventEnvelope, EventSource};
use quecto::domain::coding_job::JobState;

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

fn ensure_status_response(world: &mut QuectoWorld) {
    if world.coding_status_response.is_none() {
        let (job_id, run_id, state) = if let Some(j) = &world.coding_job {
            (j.job_id.clone(), j.run_id.clone(), j.state)
        } else {
            (
                "job_abc123".to_string(),
                "run_abc123".to_string(),
                JobState::Running,
            )
        };
        world.coding_status_response = Some(StatusResponse {
            job_id,
            run_id,
            state,
            summary: Some("status".to_string()),
            progress: None,
            todos: vec![],
            artifacts: vec![],
            error_code: None,
            error_detail: None,
            cancel_reason: None,
        });
    }
}

fn todos_mut(world: &mut QuectoWorld) -> &mut Vec<TodoItem> {
    ensure_status_response(world);
    &mut world
        .coding_status_response
        .as_mut()
        .expect("status response")
        .todos
}

fn find_todo<'a>(world: &'a QuectoWorld, todo_id: &str) -> &'a TodoItem {
    world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos
        .iter()
        .find(|t| t.todo_id == todo_id)
        .expect("todo not found")
}

fn can_transition(from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    match from {
        "pending" => matches!(to, "in_progress" | "blocked" | "canceled"),
        "in_progress" => matches!(to, "blocked" | "completed" | "failed" | "canceled"),
        "blocked" => matches!(to, "in_progress" | "failed" | "canceled"),
        "completed" | "failed" | "canceled" => false,
        _ => false,
    }
}

fn emit(world: &mut QuectoWorld, event_type: &str, payload: serde_json::Value) {
    let seq = world.coding_events.len() as u64 + 1;
    let (run_id, job_id) = if let Some(j) = &world.coding_job {
        (j.run_id.clone(), j.job_id.clone())
    } else {
        ("run_abc123".to_string(), "job_abc123".to_string())
    };
    world.coding_events.push(EventEnvelope {
        v: "1.0".to_string(),
        ts: "2026-01-01T00:00:00Z".to_string(),
        run_id,
        job_id,
        source: EventSource::Worker,
        event_type: event_type.to_string(),
        seq,
        payload,
    });
}

fn apply_todo_create(world: &mut QuectoWorld, fields: &serde_json::Map<String, serde_json::Value>) {
    world.coding_todo_create_rejected = false;
    ensure_status_response(world);

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
        .expect("status required")
        .to_string();

    if status != "pending" {
        world.coding_todo_create_rejected = true;
        return;
    }

    let max_items = world.coding_todo_max_items_per_job;
    let todos = todos_mut(world);
    if todos.iter().any(|t| t.todo_id == todo_id) {
        world.coding_todo_create_rejected = true;
        return;
    }
    if let Some(limit) = max_items
        && todos.len() >= limit
    {
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

    todos.push(TodoItem {
        todo_id: todo_id.clone(),
        title: title.clone(),
        status: "pending".to_string(),
        owner: owner.clone(),
        depends_on: depends_on.clone(),
        artifact_refs: vec![],
    });

    let mut payload = serde_json::json!({"todo_id": todo_id, "title": title, "status": "pending"});
    if let Some(owner) = owner {
        payload["owner"] = serde_json::Value::String(owner);
    }
    if !depends_on.is_empty() {
        payload["depends_on"] = serde_json::json!(depends_on);
    }
    emit(world, "todo.create", payload);
}

fn apply_todo_update(world: &mut QuectoWorld, todo_id: &str, status: &str, note: Option<String>) {
    world.coding_todo_transition_rejected = false;
    let todos = todos_mut(world);
    let todo = todos
        .iter_mut()
        .find(|t| t.todo_id == todo_id)
        .expect("todo not found");

    if !can_transition(&todo.status, status) {
        world.coding_todo_transition_rejected = true;
        return;
    }

    todo.status = status.to_string();
    if let Some(note_text) = note.clone() {
        world
            .coding_todo_notes
            .insert(todo_id.to_string(), note_text.clone());
    }

    let mut payload = serde_json::json!({"todo_id": todo_id, "status": status});
    if let Some(note_text) = note {
        payload["note"] = serde_json::Value::String(note_text);
    }
    emit(world, "todo.update", payload);
}

fn apply_todo_complete(
    world: &mut QuectoWorld,
    todo_id: &str,
    result: Option<String>,
    artifact_refs: Option<Vec<String>>,
) {
    world.coding_todo_transition_rejected = false;
    let todos = todos_mut(world);
    let todo = todos
        .iter_mut()
        .find(|t| t.todo_id == todo_id)
        .expect("todo not found");

    if !can_transition(&todo.status, "completed") {
        world.coding_todo_transition_rejected = true;
        return;
    }

    todo.status = "completed".to_string();
    if let Some(refs) = artifact_refs.clone() {
        todo.artifact_refs = refs.clone();
    }
    if let Some(res) = result.clone() {
        world
            .coding_todo_results
            .insert(todo_id.to_string(), res.clone());
    }

    let mut payload = serde_json::json!({"todo_id": todo_id});
    if let Some(res) = result {
        payload["result"] = serde_json::Value::String(res);
    }
    if let Some(refs) = artifact_refs {
        payload["artifact_refs"] = serde_json::json!(refs);
    }
    emit(world, "todo.complete", payload);
}

fn apply_todo_blocked(world: &mut QuectoWorld, todo_id: &str, reason: &str, needs: Option<String>) {
    world.coding_todo_transition_rejected = false;
    let todos = todos_mut(world);
    let todo = todos
        .iter_mut()
        .find(|t| t.todo_id == todo_id)
        .expect("todo not found");

    if !can_transition(&todo.status, "blocked") {
        world.coding_todo_transition_rejected = true;
        return;
    }

    todo.status = "blocked".to_string();
    world
        .coding_todo_blocked_reasons
        .insert(todo_id.to_string(), reason.to_string());
    if let Some(needs_text) = needs.clone() {
        world
            .coding_todo_blocked_needs
            .insert(todo_id.to_string(), needs_text.clone());
    }

    let mut payload = serde_json::json!({"todo_id": todo_id, "reason": reason});
    if let Some(needs_text) = needs {
        payload["needs"] = serde_json::Value::String(needs_text);
    }
    emit(world, "todo.blocked", payload);
}

#[given(expr = "the job has todo {string} with status {string}")]
fn given_job_has_todo(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todos = todos_mut(world);
    if todos.iter().any(|t| t.todo_id == todo_id) {
        return;
    }
    todos.push(TodoItem {
        todo_id,
        title: "test todo".to_string(),
        status,
        owner: None,
        depends_on: vec![],
        artifact_refs: vec![],
    });
}

#[given("the job has todos:")]
fn given_job_has_todos(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("table expected");
    let todos = todos_mut(world);
    todos.clear();

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

        todos.push(TodoItem {
            todo_id,
            title,
            status,
            owner: None,
            depends_on,
            artifact_refs: vec![],
        });
    }
}

#[given(expr = "the coordinator is configured with max_items_per_job {int}")]
fn given_max_items(world: &mut QuectoWorld, max_items: usize) {
    world.coding_todo_max_items_per_job = Some(max_items);
}

#[given(expr = "the job already has {int} todo items")]
fn given_job_already_has_n_todos(world: &mut QuectoWorld, count: usize) {
    let todos = todos_mut(world);
    todos.clear();
    for i in 0..count {
        todos.push(TodoItem {
            todo_id: format!("t{}", i + 1),
            title: format!("Todo {}", i + 1),
            status: "pending".to_string(),
            owner: None,
            depends_on: vec![],
            artifact_refs: vec![],
        });
    }
}

#[when(expr = "the worker emits a {string} event with:")]
fn when_worker_emits_todo_event_with_table(
    world: &mut QuectoWorld,
    event_type: String,
    step: &gherkin::Step,
) {
    let fields = parse_kv_table(step);
    if event_type == "todo.create" {
        apply_todo_create(world, &fields);
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
        apply_todo_update(world, &todo_id, &status, None);
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
        apply_todo_complete(world, &todo_id, Some(result), None);
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
        apply_todo_complete(world, &todo_id, None, Some(parse_list_literal(&refs)));
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
        apply_todo_blocked(world, &todo_id, &reason, None);
    }
}

#[when(expr = "the worker emits a {string} event for {string} with:")]
fn when_worker_emits_todo_event_for_with_table(
    world: &mut QuectoWorld,
    event_type: String,
    todo_id: String,
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
        apply_todo_blocked(world, &todo_id, reason, needs);
    } else if event_type == "todo.update" {
        let status = fields
            .get("status")
            .and_then(|v| v.as_str())
            .expect("status required");
        let note = fields
            .get("note")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        apply_todo_update(world, &todo_id, status, note);
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
        apply_todo_create(world, &fields);
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
    apply_todo_create(world, &fields);
}

#[when("the worker creates and completes a todo")]
fn when_worker_creates_and_completes_todo(world: &mut QuectoWorld) {
    let create = serde_json::Map::from_iter(vec![
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
    apply_todo_create(world, &create);
    apply_todo_update(world, "t1", "in_progress", None);
    apply_todo_complete(world, "t1", Some("done".to_string()), None);
}

#[when("the parent job is canceled")]
fn when_parent_job_canceled(world: &mut QuectoWorld) {
    let todos = todos_mut(world);
    for todo in todos.iter_mut() {
        if matches!(todo.status.as_str(), "pending" | "in_progress" | "blocked") {
            todo.status = "canceled".to_string();
        }
    }
}

#[then(expr = "the coordinator should record todo {string} with status {string}")]
fn then_recorded_todo_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then(expr = "the job's todo list should contain {int} item")]
fn then_todo_list_contains_one(world: &mut QuectoWorld, count: usize) {
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos;
    assert_eq!(todos.len(), count);
}

#[then(expr = "todo {string} should have depends_on containing {string}")]
fn then_todo_depends_on(world: &mut QuectoWorld, todo_id: String, dep: String) {
    let todo = find_todo(world, &todo_id);
    assert!(todo.depends_on.contains(&dep));
}

#[then(expr = "todo {string} should have status {string}")]
fn then_todo_has_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then(expr = "the completion result should be {string}")]
fn then_completion_result(world: &mut QuectoWorld, result: String) {
    assert!(world.coding_todo_results.values().any(|r| r == &result));
}

#[then(expr = "todo {string} should have artifact_refs containing {string}")]
fn then_todo_has_artifact_ref(world: &mut QuectoWorld, todo_id: String, artifact: String) {
    let todo = find_todo(world, &todo_id);
    assert!(todo.artifact_refs.contains(&artifact));
}

#[then(expr = "the blocked reason should be {string}")]
fn then_blocked_reason(world: &mut QuectoWorld, reason: String) {
    assert!(
        world
            .coding_todo_blocked_reasons
            .values()
            .any(|r| r == &reason)
    );
}

#[then(expr = "the blocked event should include needs {string}")]
fn then_blocked_event_needs(world: &mut QuectoWorld, needs: String) {
    let event = world
        .coding_events
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
        .expect("status response")
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
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos;
    assert_eq!(todos.len(), count);
}

#[then(expr = "the job should still have {int} todo item")]
fn then_job_still_has_todo_count_singular(world: &mut QuectoWorld, count: usize) {
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos;
    assert_eq!(todos.len(), count);
}

#[then("the event log should contain both \"todo.create\" and \"todo.complete\" events")]
fn then_event_log_contains_create_and_complete(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_events
            .iter()
            .any(|e| e.event_type == "todo.create")
    );
    assert!(
        world
            .coding_events
            .iter()
            .any(|e| e.event_type == "todo.complete")
    );
}

#[then("the events should have correct envelope fields")]
fn then_events_have_envelope_fields(world: &mut QuectoWorld) {
    for e in &world.coding_events {
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
    let todos = &world
        .coding_status_response
        .as_ref()
        .expect("status response")
        .todos;
    for todo in todos {
        if matches!(todo.status.as_str(), "pending" | "in_progress" | "blocked") {
            panic!("todo {} should have been canceled", todo.todo_id);
        }
    }
}

#[then(expr = "todo {string} should have owner {string}")]
fn then_todo_owner(world: &mut QuectoWorld, todo_id: String, owner: String) {
    let todo = find_todo(world, &todo_id);
    assert_eq!(todo.owner.as_deref(), Some(owner.as_str()));
}

#[then(expr = "todo {string} should have the note {string}")]
fn then_todo_note(world: &mut QuectoWorld, todo_id: String, note: String) {
    assert_eq!(world.coding_todo_notes.get(&todo_id), Some(&note));
}

#[then("the coordinator should reject the transition")]
fn then_transition_rejected(world: &mut QuectoWorld) {
    assert!(world.coding_todo_transition_rejected);
}

#[then(expr = "todo {string} should remain in status {string}")]
fn then_todo_remains_status(world: &mut QuectoWorld, todo_id: String, status: String) {
    let todo = find_todo(world, &todo_id);
    assert_eq!(todo.status, status);
}

#[then("the coordinator should reject the create as duplicate")]
fn then_reject_duplicate_create(world: &mut QuectoWorld) {
    assert!(world.coding_todo_create_rejected);
}
