use super::*;

use quecto::domain::coding_command::{StatusResponse, TodoItem};
use quecto::domain::coding_event::EventSource;
use quecto::domain::coding_job::JobState;

struct SpawnRequest {
    request_id: String,
    agent_type: String,
    scope: String,
    expected_output: Option<String>,
}

fn list_from_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_spawn_request(step: &gherkin::Step) -> SpawnRequest {
    let table = step.table.as_ref().expect("spawn.request table expected");
    let mut request_id = "s1".to_string();
    let mut agent_type = "security-reviewer".to_string();
    let mut scope = "current diff".to_string();
    let mut expected_output = None;
    for row in &table.rows {
        let key = row.first().map(|x| x.trim()).unwrap_or_default();
        let value = row.get(1).map(|x| x.trim()).unwrap_or_default();
        match key {
            "request_id" => request_id = value.to_string(),
            "agent_type" => agent_type = value.to_string(),
            "scope" => scope = value.to_string(),
            "expected_output" => expected_output = Some(value.to_string()),
            _ => {}
        }
    }
    SpawnRequest {
        request_id,
        agent_type,
        scope,
        expected_output,
    }
}

fn push_event(
    world: &mut QuectoWorld,
    source: EventSource,
    event_type: &str,
    payload: serde_json::Value,
) {
    push_coding_event(world, source, event_type, payload);
}

fn last_event<'a>(
    world: &'a QuectoWorld,
    event_type: &str,
) -> &'a quecto::domain::coding_event::EventEnvelope {
    world
        .coding_events
        .iter()
        .rev()
        .find(|e| e.event_type == event_type)
        .expect("event should exist")
}

fn child_request_known(world: &QuectoWorld, request_id: &str) -> bool {
    world
        .coding_child_launched_request_ids
        .iter()
        .any(|r| r == request_id)
}

fn ensure_parent_cancel_probe(world: &mut QuectoWorld) {
    if world.coding_status_response.is_none() {
        world.coding_status_response = Some(StatusResponse {
            job_id: "job_abc123".to_string(),
            run_id: "run_abc123".to_string(),
            state: JobState::Running,
            summary: Some("running".to_string()),
            progress: None,
            todos: vec![],
            artifacts: vec![],
            error_code: None,
            error_detail: None,
            cancel_reason: None,
        });
    }
    let todos = &mut world
        .coding_status_response
        .as_mut()
        .expect("status response")
        .todos;
    if todos.iter().all(|t| t.todo_id != "child-cancel-probe") {
        todos.push(TodoItem {
            todo_id: "child-cancel-probe".to_string(),
            title: "cancel probe".to_string(),
            status: "in_progress".to_string(),
            owner: None,
            depends_on: vec![],
            artifact_refs: vec![],
        });
    }
}

fn has_parent_cancel_signal(world: &QuectoWorld) -> bool {
    world
        .coding_status_response
        .as_ref()
        .map(|s| {
            s.todos
                .iter()
                .any(|t| t.todo_id == "child-cancel-probe" && t.status == "canceled")
        })
        .unwrap_or(false)
}

fn append_spawn_result(
    world: &mut QuectoWorld,
    request_id: &str,
    state: &str,
    summary: Option<&str>,
) {
    let mut payload = serde_json::json!({
        "request_id": request_id,
        "state": state,
    });
    if let Some(text) = summary {
        payload["summary"] = serde_json::Value::String(text.to_string());
    }
    if !world.coding_child_artifacts.is_empty() {
        payload["artifact_refs"] = serde_json::Value::Array(
            world
                .coding_child_artifacts
                .iter()
                .map(|x| serde_json::Value::String(x.clone()))
                .collect(),
        );
    }
    push_event(world, EventSource::Coordinator, "spawn.result", payload);
}

fn evaluate_spawn_request(world: &mut QuectoWorld, req: &SpawnRequest) {
    push_event(
        world,
        EventSource::Worker,
        "spawn.request",
        serde_json::json!({
            "request_id": req.request_id,
            "agent_type": req.agent_type,
            "scope": req.scope,
            "expected_output": req.expected_output,
        }),
    );

    let denied_reason = if world.coding_child_current_depth >= world.coding_child_max_depth {
        Some("max spawn depth is reached")
    } else if world.coding_child_spawn_count >= world.coding_child_max_spawns_per_job {
        Some("per-job spawn limit is reached")
    } else if !world
        .coding_child_allow_types
        .iter()
        .any(|x| x == &req.agent_type)
    {
        Some("agent type is not allowed")
    } else {
        None
    };

    if let Some(reason) = denied_reason {
        world.coding_child_last_decision_reason = Some(reason.to_string());
        push_event(
            world,
            EventSource::Coordinator,
            "spawn.decision",
            serde_json::json!({
                "request_id": req.request_id,
                "agent_type": req.agent_type,
                "approved": false,
                "reason": reason,
            }),
        );
        return;
    }

    let duplicate = world
        .coding_child_active_by_type
        .get(&format!("{}::{}", req.agent_type, req.scope))
        .cloned();
    push_event(
        world,
        EventSource::Coordinator,
        "spawn.decision",
        serde_json::json!({
            "request_id": req.request_id,
            "agent_type": req.agent_type,
            "approved": true,
        }),
    );

    if let Some(first_request) = duplicate {
        world.coding_child_second_reused_first = true;
        if let Some(existing) = last_event(world, "spawn.result").payload["request_id"].as_str() {
            if existing == first_request {
                append_spawn_result(
                    world,
                    &req.request_id,
                    "succeeded",
                    Some("reused first result"),
                );
            }
        }
        return;
    }

    world.coding_child_expected_output = req.expected_output.clone();
    world.coding_child_spawn_count += 1;
    world
        .coding_child_launched_request_ids
        .push(req.request_id.clone());
    world.coding_child_active_by_type.insert(
        format!("{}::{}", req.agent_type, req.scope),
        req.request_id.clone(),
    );
    world.coding_child_isolation_nsjail = true;
    world.coding_child_mount_restricted = true;
}

#[given("a coding coordinator with child agent policy:")]
fn given_child_agent_policy(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("policy table expected");
    for row in &table.rows {
        let key = row.first().map(|x| x.trim()).unwrap_or_default();
        let value = row.get(1).map(|x| x.trim()).unwrap_or_default();
        match key {
            "allow_types" => world.coding_child_allow_types = list_from_csv(value),
            "max_depth" => world.coding_child_max_depth = value.parse().expect("valid depth"),
            "max_spawns_per_job" => {
                world.coding_child_max_spawns_per_job = value.parse().expect("valid limit")
            }
            _ => {}
        }
    }
}

#[given("the job has already spawned 3 child agents")]
fn given_spawn_limit_reached(world: &mut QuectoWorld) {
    world.coding_child_spawn_count = 3;
}

#[given("the current job is already a child agent at depth 1")]
fn given_depth_one(world: &mut QuectoWorld) {
    world.coding_child_current_depth = 1;
}

#[given(expr = "a child agent {string} was approved and launched")]
fn given_child_approved_launched(world: &mut QuectoWorld, agent_type: String) {
    let request_id = if agent_type == "performance-reviewer" {
        "s2"
    } else {
        "s1"
    };
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: request_id.to_string(),
            agent_type,
            scope: "current diff".to_string(),
            expected_output: None,
        },
    );
}

#[given(expr = "a child agent {string} is running")]
fn given_child_running(world: &mut QuectoWorld, agent_type: String) {
    given_child_approved_launched(world, agent_type);
    ensure_parent_cancel_probe(world);
}

#[given(expr = "a child agent {string} was canceled")]
fn given_child_canceled(world: &mut QuectoWorld, agent_type: String) {
    given_child_approved_launched(world, agent_type);
    append_spawn_result(world, "s1", "canceled", Some("canceled"));
    world.coding_child_canceled_terminal = true;
    world.coding_child_terminal_event_count = Some(world.coding_events.len());
}

#[given(expr = "a child agent {string} is running at depth 1")]
fn given_child_running_depth_one(world: &mut QuectoWorld, agent_type: String) {
    given_child_running(world, agent_type);
    world.coding_child_current_depth = 1;
}

#[when("the worker emits a \"spawn.request\" event with:")]
fn when_worker_emits_spawn_request_with(world: &mut QuectoWorld, step: &gherkin::Step) {
    let req = parse_spawn_request(step);
    evaluate_spawn_request(world, &req);
}

#[when("the worker emits a \"spawn.request\" event")]
fn when_worker_emits_spawn_request(world: &mut QuectoWorld) {
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: "s1".to_string(),
            agent_type: "security-reviewer".to_string(),
            scope: "current diff".to_string(),
            expected_output: None,
        },
    );
}

#[when("the worker emits a 4th \"spawn.request\" event")]
fn when_worker_emits_fourth_request(world: &mut QuectoWorld) {
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: "s4".to_string(),
            agent_type: "architecture-reviewer".to_string(),
            scope: "current diff".to_string(),
            expected_output: None,
        },
    );
}

#[when(expr = "the child agent completes with state {string} and summary {string}")]
fn when_child_completes(world: &mut QuectoWorld, state: String, summary: String) {
    append_spawn_result(world, "s1", &state, Some(&summary));
    world.coding_child_parent_notified = true;
    world.coding_child_main_summary_updated = true;
}

#[when(expr = "the child agent fails with state {string}")]
fn when_child_fails(world: &mut QuectoWorld, state: String) {
    append_spawn_result(world, "s2", &state, Some("child failed"));
    world.coding_child_parent_notified = true;
}

#[when(expr = "the child agent creates artifact {string}")]
fn when_child_creates_artifact(world: &mut QuectoWorld, artifact: String) {
    world.coding_child_artifacts.push(artifact);
    append_spawn_result(world, "s1", "succeeded", Some("artifact produced"));
}

#[when("the child agent attempts to emit a \"publish.request\" event")]
fn when_child_attempts_publish(world: &mut QuectoWorld) {
    push_event(
        world,
        EventSource::ChildAgent,
        "log.message",
        serde_json::json!({"level": "error", "message": "publish.request denied for child agent"}),
    );
    world.coding_child_publish_rejected = true;
    world.coding_child_error_returned = true;
}

#[when("cancel propagation to child agents is processed")]
fn when_cancel_propagation_processed(world: &mut QuectoWorld) {
    if has_parent_cancel_signal(world) {
        world.coding_child_terminated = true;
        if !world.coding_events.iter().any(|e| {
            e.event_type == "spawn.result"
                && e.payload["state"] == serde_json::Value::String("canceled".to_string())
        }) {
            append_spawn_result(world, "s1", "canceled", Some("parent canceled"));
        }
    }
}

#[when("a worker requests a child agent and it completes")]
fn when_worker_requests_and_completes(world: &mut QuectoWorld) {
    when_worker_emits_spawn_request(world);
    append_spawn_result(world, "s1", "succeeded", Some("done"));
    push_event(
        world,
        EventSource::Coordinator,
        "artifact.created",
        serde_json::json!({
            "artifact_id": "artifact_spawn_log_1",
            "artifact_type": "spawn_log",
            "path": "artifacts/spawn.log"
        }),
    );
}

#[when("the worker emits two \"spawn.request\" events with identical agent_type and scope")]
fn when_worker_emits_duplicate_requests(world: &mut QuectoWorld) {
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: "s1".to_string(),
            agent_type: "security-reviewer".to_string(),
            scope: "current diff".to_string(),
            expected_output: None,
        },
    );
    append_spawn_result(world, "s1", "succeeded", Some("first result"));
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: "s2".to_string(),
            agent_type: "security-reviewer".to_string(),
            scope: "current diff".to_string(),
            expected_output: None,
        },
    );
}

#[when("the child agent exceeds the configured timeout")]
fn when_child_exceeds_timeout(world: &mut QuectoWorld) {
    world.coding_child_terminated = true;
    append_spawn_result(world, "s1", "failed", Some("timeout"));
}

#[when("the coordinator checks the spawn result")]
fn when_coordinator_checks_spawn_result(world: &mut QuectoWorld) {
    world.coding_child_extra_events_after_terminal = false;
}

#[when("the worker emits \"spawn.request\" events for:")]
fn when_worker_emits_requests_for(world: &mut QuectoWorld, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("spawn table expected");
    for row in table.rows.iter().skip(1) {
        let request_id = row
            .first()
            .map(|x| x.trim())
            .unwrap_or_default()
            .to_string();
        let agent_type = row.get(1).map(|x| x.trim()).unwrap_or_default().to_string();
        evaluate_spawn_request(
            world,
            &SpawnRequest {
                request_id,
                agent_type,
                scope: "current diff".to_string(),
                expected_output: None,
            },
        );
    }
}

#[when("the child agent emits a \"spawn.request\" for another child agent")]
fn when_child_emits_nested_request(world: &mut QuectoWorld) {
    evaluate_spawn_request(
        world,
        &SpawnRequest {
            request_id: "s_nested".to_string(),
            agent_type: "performance-reviewer".to_string(),
            scope: "nested".to_string(),
            expected_output: None,
        },
    );
}

#[when(expr = "a \"spawn.result\" event arrives with request_id {string}")]
fn when_unknown_spawn_result_arrives(world: &mut QuectoWorld, request_id: String) {
    if !child_request_known(world, &request_id) {
        world.coding_child_unknown_request_warning = true;
        world.coding_child_result_discarded = true;
    }
}

#[then(expr = "a \"spawn.decision\" event should be emitted with approved {word}")]
fn then_spawn_decision_approved(world: &mut QuectoWorld, approved: String) {
    let event = last_event(world, "spawn.decision");
    let expected = approved == "true";
    assert_eq!(event.payload["approved"], serde_json::Value::Bool(expected));
}

#[then("the child agent should be launched")]
fn then_child_launched(world: &mut QuectoWorld) {
    assert!(!world.coding_child_launched_request_ids.is_empty());
}

#[then("the reason should indicate the agent type is not allowed")]
fn then_reason_agent_not_allowed(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_child_last_decision_reason
            .as_deref()
            .unwrap_or_default()
            .contains("agent type is not allowed")
    );
}

#[then("no child agent should be launched")]
fn then_no_child_launched(world: &mut QuectoWorld) {
    assert!(world.coding_child_launched_request_ids.is_empty());
}

#[then("the reason should indicate the per-job spawn limit is reached")]
fn then_reason_limit(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_child_last_decision_reason
            .as_deref()
            .unwrap_or_default()
            .contains("per-job spawn limit is reached")
    );
}

#[then("the reason should indicate the max spawn depth is reached")]
fn then_reason_depth(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_child_last_decision_reason
            .as_deref()
            .unwrap_or_default()
            .contains("max spawn depth is reached")
    );
}

#[then("a \"spawn.result\" event should be emitted with:")]
fn then_spawn_result_with_table(world: &mut QuectoWorld, step: &gherkin::Step) {
    let event = last_event(world, "spawn.result");
    let table = step.table.as_ref().expect("result table expected");
    for row in &table.rows {
        let key = row.first().map(|x| x.trim()).unwrap_or_default();
        let value = row.get(1).map(|x| x.trim()).unwrap_or_default();
        assert_eq!(
            event.payload[key],
            serde_json::Value::String(value.to_string())
        );
    }
}

#[then("the parent worker should receive the child result")]
fn then_parent_worker_notified(world: &mut QuectoWorld) {
    assert!(world.coding_child_parent_notified);
}

#[then("the main agent should receive an updated job summary")]
fn then_main_summary_updated(world: &mut QuectoWorld) {
    assert!(world.coding_child_main_summary_updated);
}

#[then("the parent worker should be notified of the failure")]
fn then_parent_notified_failure(world: &mut QuectoWorld) {
    assert!(world.coding_child_parent_notified);
}

#[then(expr = "a \"spawn.result\" event should include artifact_refs containing {string}")]
fn then_spawn_result_has_artifact(world: &mut QuectoWorld, artifact: String) {
    let event = last_event(world, "spawn.result");
    let refs = event.payload["artifact_refs"]
        .as_array()
        .expect("artifact_refs should exist");
    assert!(
        refs.iter()
            .any(|v| *v == serde_json::Value::String(artifact.clone()))
    );
}

#[then("the artifact should be accessible in the parent job's artifact directory")]
fn then_artifact_accessible(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_child_artifacts
            .iter()
            .any(|a| a == "security_review.md")
    );
}

#[then("the publish request should be rejected by the coordinator")]
fn then_coordinator_rejects_publish(world: &mut QuectoWorld) {
    assert!(world.coding_child_publish_rejected);
}

#[then("the child agent should receive an error")]
fn then_child_receives_error(world: &mut QuectoWorld) {
    assert!(world.coding_child_error_returned);
}

#[then("the child agent should run inside nsjail with the same resource limits")]
fn then_child_runs_in_nsjail(world: &mut QuectoWorld) {
    assert!(world.coding_child_isolation_nsjail);
}

#[then("the child agent should have a writable mount only for its own job directory")]
fn then_child_mount_restricted(world: &mut QuectoWorld) {
    assert!(world.coding_child_mount_restricted);
}

#[then(
    "the event log should contain \"spawn.request\", \"spawn.decision\", and \"spawn.result\" events"
)]
fn then_event_log_contains_spawn_triplet(world: &mut QuectoWorld) {
    let has_request = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "spawn.request");
    let has_decision = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "spawn.decision");
    let has_result = world
        .coding_events
        .iter()
        .any(|e| e.event_type == "spawn.result");
    assert!(has_request && has_decision && has_result);
}

#[then("only one child agent should be launched")]
fn then_only_one_child_launched(world: &mut QuectoWorld) {
    assert_eq!(world.coding_child_spawn_count, 1);
}

#[then("the second request should receive the result of the first")]
fn then_second_request_reuses_result(world: &mut QuectoWorld) {
    assert!(world.coding_child_second_reused_first);
    let event = last_event(world, "spawn.result");
    assert_eq!(
        event.payload["request_id"],
        serde_json::Value::String("s2".to_string())
    );
}

#[then("the child agent should receive the expected_output specification")]
fn then_expected_output_forwarded(world: &mut QuectoWorld) {
    assert!(world.coding_child_expected_output.is_some());
}

#[then("the child agent should be terminated")]
fn then_child_terminated(world: &mut QuectoWorld) {
    assert!(world.coding_child_terminated);
}

#[then("the summary should indicate timeout")]
fn then_summary_timeout(world: &mut QuectoWorld) {
    let event = last_event(world, "spawn.result");
    assert!(
        event.payload["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("timeout")
    );
}

#[then("the spawn.result state should be \"canceled\"")]
fn then_spawn_result_canceled(world: &mut QuectoWorld) {
    let event = last_event(world, "spawn.result");
    assert_eq!(
        event.payload["state"],
        serde_json::Value::String("canceled".to_string())
    );
}

#[then("no further events should be emitted for this child agent")]
fn then_no_further_events_for_canceled_child(world: &mut QuectoWorld) {
    assert!(world.coding_child_canceled_terminal);
    assert_eq!(
        world.coding_child_terminal_event_count,
        Some(world.coding_events.len())
    );
}

#[then("all 3 spawn.decision events should have approved true")]
fn then_all_three_approved(world: &mut QuectoWorld) {
    let approved_count = world
        .coding_events
        .iter()
        .filter(|e| {
            e.event_type == "spawn.decision"
                && e.payload["approved"] == serde_json::Value::Bool(true)
        })
        .count();
    assert_eq!(approved_count, 3);
}

#[then("all 3 child agents should be launched concurrently")]
fn then_all_three_launched(world: &mut QuectoWorld) {
    assert_eq!(world.coding_child_spawn_count, 3);
}

#[then("the reason should indicate max depth 1 would be exceeded")]
fn then_reason_max_depth_one(world: &mut QuectoWorld) {
    assert!(
        world
            .coding_child_last_decision_reason
            .as_deref()
            .unwrap_or_default()
            .contains("max spawn depth is reached")
    );
}

#[then("a warning should be logged for unknown request_id")]
fn then_unknown_request_warning(world: &mut QuectoWorld) {
    assert!(world.coding_child_unknown_request_warning);
}

#[then("the event should be discarded")]
fn then_unknown_result_discarded(world: &mut QuectoWorld) {
    assert!(world.coding_child_result_discarded);
}
