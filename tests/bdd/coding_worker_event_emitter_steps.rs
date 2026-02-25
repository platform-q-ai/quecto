use cucumber::{given, then, when};
use quecto::infrastructure::coding::worker_event_emitter::{EmitterConfig, WorkerEventEmitter};

use crate::QuectoWorld;

// ── Helpers ─────────────────────────────────────────────────────────────

fn emitter_ref(world: &mut QuectoWorld) -> &mut WorkerEventEmitter<Vec<u8>> {
    world.wee_emitter.as_mut().expect("emitter not set")
}

fn emitter_output(world: &QuectoWorld) -> String {
    let emitter = world.wee_emitter.as_ref().expect("emitter not set");
    String::from_utf8(emitter.writer().clone()).unwrap()
}

fn last_emitted_json(world: &QuectoWorld) -> serde_json::Value {
    let output = emitter_output(world);
    let last_line = output.lines().last().expect("no emitted lines");
    serde_json::from_str(last_line).expect("invalid JSON")
}

fn payload_from_table(step: &cucumber::gherkin::Step) -> serde_json::Value {
    let table = step.table.as_ref().expect("expected a table");
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            let key = row[0].trim().to_string();
            let val = row[1].trim().to_string();
            // Try to parse as number or bool, fall back to string
            if val == "true" {
                map.insert(key, serde_json::Value::Bool(true));
            } else if val == "false" {
                map.insert(key, serde_json::Value::Bool(false));
            } else if let Ok(n) = val.parse::<i64>() {
                map.insert(key, serde_json::json!(n));
            } else {
                map.insert(key, serde_json::Value::String(val));
            }
        }
    }
    serde_json::Value::Object(map)
}

// ── Background ──────────────────────────────────────────────────────────

#[given(expr = "a worker event emitter for run {string} and job {string}")]
fn given_emitter(world: &mut QuectoWorld, run_id: String, job_id: String) {
    let emitter = WorkerEventEmitter::new(
        EmitterConfig {
            run_id,
            job_id,
            version: "1.0".to_string(),
        },
        Vec::new(),
    );
    world.wee_emitter = Some(emitter);
}

#[given("a worker event emitter writing to a buffer")]
fn given_emitter_with_buffer(world: &mut QuectoWorld) {
    let emitter = WorkerEventEmitter::new(
        EmitterConfig {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            version: "1.0".to_string(),
        },
        Vec::new(),
    );
    world.wee_emitter = Some(emitter);
}

// ── When steps ──────────────────────────────────────────────────────────

#[when(expr = "the worker emits a {string} event with payload:")]
fn when_emit_event(world: &mut QuectoWorld, event_type: String, step: &cucumber::gherkin::Step) {
    let payload = payload_from_table(step);
    let emitter = emitter_ref(world);
    let result = emitter.emit(&event_type, payload);
    world.wee_last_emit_result = Some(result.map_err(|e| e.to_string()));
}

#[when(expr = "the worker emits {int} {string} events")]
fn when_emit_multiple(world: &mut QuectoWorld, count: usize, event_type: String) {
    let emitter = emitter_ref(world);
    for i in 0..count {
        let payload = serde_json::json!({
            "level": "info",
            "message": format!("event {i}")
        });
        emitter.emit(&event_type, payload).unwrap();
    }
}

#[when(expr = "the worker tries to emit an event with type {string}")]
fn when_try_emit_unknown(world: &mut QuectoWorld, event_type: String) {
    let emitter = emitter_ref(world);
    let result = emitter.emit(&event_type, serde_json::json!({}));
    world.wee_last_emit_result = Some(result.map_err(|e| e.to_string()));
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then("the emitted line should be valid JSON")]
fn then_emitted_valid_json(world: &mut QuectoWorld) {
    let output = emitter_output(world);
    let last_line = output.lines().last().expect("no lines emitted");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(last_line);
    assert!(parsed.is_ok(), "line is not valid JSON: {last_line}");
}

#[then(expr = "the emitted event should have version matching {string}")]
fn then_event_version(world: &mut QuectoWorld, expected: String) {
    let json = last_emitted_json(world);
    assert_eq!(json["v"].as_str().unwrap(), expected, "version mismatch");
}

#[then(expr = "the emitted event should have source {string}")]
fn then_event_source(world: &mut QuectoWorld, expected: String) {
    let json = last_emitted_json(world);
    assert_eq!(
        json["source"].as_str().unwrap(),
        expected,
        "source mismatch"
    );
}

#[then(expr = "the emitted event should have run_id {string}")]
fn then_event_run_id(world: &mut QuectoWorld, expected: String) {
    let json = last_emitted_json(world);
    assert_eq!(
        json["run_id"].as_str().unwrap(),
        expected,
        "run_id mismatch"
    );
}

#[then(expr = "the emitted event should have job_id {string}")]
fn then_event_job_id(world: &mut QuectoWorld, expected: String) {
    let json = last_emitted_json(world);
    assert_eq!(
        json["job_id"].as_str().unwrap(),
        expected,
        "job_id mismatch"
    );
}

#[then(expr = "the emitted event should have type {string}")]
fn then_event_type(world: &mut QuectoWorld, expected: String) {
    let json = last_emitted_json(world);
    assert_eq!(
        json["type"].as_str().unwrap(),
        expected,
        "event type mismatch"
    );
}

#[then(expr = "the emitted event should have seq {int}")]
fn then_event_seq(world: &mut QuectoWorld, expected: u64) {
    let json = last_emitted_json(world);
    assert_eq!(json["seq"].as_u64().unwrap(), expected, "seq mismatch");
}

#[then(expr = "the last emitted event should have seq {int}")]
fn then_last_event_seq(world: &mut QuectoWorld, expected: u64) {
    let json = last_emitted_json(world);
    assert_eq!(json["seq"].as_u64().unwrap(), expected, "last seq mismatch");
}

#[then(expr = "the emitted event payload should have {string} equal to {string}")]
fn then_payload_field_eq(world: &mut QuectoWorld, field: String, expected: String) {
    let json = last_emitted_json(world);
    let payload = &json["payload"];
    let actual = &payload[&field];
    // Compare as string regardless of JSON type
    let actual_str = match actual {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    };
    assert_eq!(actual_str, expected, "payload.{field} mismatch");
}

#[then("the emitted event timestamp should match ISO 8601 format")]
fn then_timestamp_iso8601(world: &mut QuectoWorld) {
    let json = last_emitted_json(world);
    let ts = json["ts"].as_str().expect("missing ts");
    assert!(ts.contains('T'), "timestamp should contain T: {ts}");
    assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
}

#[then(expr = "{int} lines should have been emitted")]
fn then_n_lines_emitted(world: &mut QuectoWorld, expected: usize) {
    let output = emitter_output(world);
    let count = output.lines().count();
    assert_eq!(count, expected, "expected {expected} lines but got {count}");
}

#[then("each emitted line should be valid JSON")]
fn then_each_line_valid_json(world: &mut QuectoWorld) {
    let output = emitter_output(world);
    for (i, line) in output.lines().enumerate() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "line {i} is not valid JSON: {line}");
    }
}

#[then("the emission should return an error")]
fn then_emission_error(world: &mut QuectoWorld) {
    let result = world.wee_last_emit_result.as_ref().expect("no result");
    assert!(result.is_err(), "expected error but got Ok");
}

#[then(expr = "the emission error should mention {string}")]
fn then_emission_error_mentions(world: &mut QuectoWorld, expected: String) {
    let result = world.wee_last_emit_result.as_ref().expect("no result");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains(&expected),
        "expected error to contain '{expected}' but got: {err}"
    );
}

#[then("the buffer should contain exactly one JSON line")]
fn then_buffer_one_line(world: &mut QuectoWorld) {
    let output = emitter_output(world);
    let count = output.lines().count();
    assert_eq!(count, 1, "expected 1 line but got {count}");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(output.lines().next().unwrap());
    assert!(parsed.is_ok(), "line is not valid JSON");
}

#[then("the raw emitted output should end with a newline")]
fn then_output_ends_newline(world: &mut QuectoWorld) {
    let output = emitter_output(world);
    assert!(output.ends_with('\n'), "output should end with newline");
}
