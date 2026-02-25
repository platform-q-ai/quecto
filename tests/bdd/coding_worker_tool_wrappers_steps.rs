use cucumber::{given, then, when};
use quecto::infrastructure::coding::worker_tool_wrappers::build_worker_tool_registry;
use tempfile::TempDir;

use crate::QuectoWorld;

// ── Background steps ────────────────────────────────────────────────────

#[given("a worker tool registry with job directory")]
fn given_worker_tool_registry(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    let job_dir = tmp.path().to_path_buf();
    let registry = build_worker_tool_registry(job_dir.clone());
    world.wtw_job_dir = Some(job_dir);
    world.wtw_registry = Some(registry);
    world._wtw_temp_dir = Some(tmp);
}

#[given(expr = "the job directory contains files:")]
fn given_job_dir_contains_files(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let job_dir = world.wtw_job_dir.as_ref().expect("job dir not set");
    let table = step.table.as_ref().expect("expected a table");
    for row in &table.rows[1..] {
        let path = row[0].trim();
        let content = row[1].trim().replace("\\n", "\n");
        let full_path = job_dir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full_path, &content).unwrap();
    }
}

// ── Given steps ─────────────────────────────────────────────────────────

#[given(expr = "the job directory contains a file {string} with content {string}")]
fn given_job_dir_has_file(world: &mut QuectoWorld, path: String, content: String) {
    let job_dir = world.wtw_job_dir.as_ref().expect("job dir not set");
    let full_path = job_dir.join(&path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&full_path, content.replace("\\n", "\n")).unwrap();
}

// ── When steps ──────────────────────────────────────────────────────────

#[when(expr = "I execute worker tool {string} with arguments:")]
fn when_execute_worker_tool(
    world: &mut QuectoWorld,
    tool_name: String,
    step: &cucumber::gherkin::Step,
) {
    let arguments = step
        .docstring
        .as_ref()
        .expect("expected a docstring")
        .trim();
    let registry = world.wtw_registry.as_ref().expect("registry not set");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(registry.execute(&tool_name, arguments));
    match result {
        Ok(tool_result) => {
            world.wtw_last_result = Some(Ok(tool_result));
        }
        Err(e) => {
            world.wtw_last_result = Some(Err(e.to_string()));
        }
    }
}

// ── Then steps ──────────────────────────────────────────────────────────

#[then(expr = "the worker tool registry should contain a tool named {string}")]
fn then_registry_contains_tool(world: &mut QuectoWorld, name: String) {
    let registry = world.wtw_registry.as_ref().expect("registry not set");
    assert!(
        registry.get(&name).is_some(),
        "expected registry to contain tool '{name}'"
    );
}

#[then(expr = "the {string} tool definition should require fields {string}")]
fn then_tool_requires_fields(world: &mut QuectoWorld, tool_name: String, fields_csv: String) {
    let registry = world.wtw_registry.as_ref().expect("registry not set");
    let tool = registry.get(&tool_name).expect("tool not found");
    let def = tool.definition();
    let schema: serde_json::Value =
        serde_json::from_str(&def.parameters_schema).expect("invalid schema JSON");
    let required = schema["required"]
        .as_array()
        .expect("missing 'required' in schema");
    for field in fields_csv.split(',').map(|f| f.trim()) {
        assert!(
            required.contains(&serde_json::json!(field)),
            "schema should require '{field}'"
        );
    }
}

#[then(expr = "the {string} tool definition should require {string}")]
fn then_tool_requires_one(world: &mut QuectoWorld, tool_name: String, field: String) {
    let registry = world.wtw_registry.as_ref().expect("registry not set");
    let tool = registry.get(&tool_name).expect("tool not found");
    let def = tool.definition();
    let schema: serde_json::Value =
        serde_json::from_str(&def.parameters_schema).expect("invalid schema JSON");
    let required = schema["required"]
        .as_array()
        .expect("missing 'required' in schema");
    assert!(
        required.contains(&serde_json::json!(field)),
        "schema should require '{field}'"
    );
}

#[then("the worker tool result should succeed")]
fn then_worker_result_ok(world: &mut QuectoWorld) {
    let result = world.wtw_last_result.as_ref().expect("no result");
    assert!(
        result.is_ok(),
        "expected Ok but got Err: {:?}",
        result.as_ref().err()
    );
    let tool_result = result.as_ref().unwrap();
    assert!(
        !tool_result.is_error,
        "tool result is_error was true: {}",
        tool_result.content
    );
}

#[then("the worker tool result should indicate an error")]
fn then_worker_result_error(world: &mut QuectoWorld) {
    let result = world.wtw_last_result.as_ref().expect("no result");
    match result {
        Err(e) => {
            assert!(!e.is_empty(), "DomainError should have a message");
        }
        Ok(tr) => {
            assert!(
                tr.is_error,
                "expected is_error=true but got false: {}",
                tr.content
            );
        }
    }
}

#[then(expr = "the worker tool result JSON should have {string} equal to true")]
fn then_wtw_json_field_true(world: &mut QuectoWorld, field: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    assert_eq!(
        json[&field], true,
        "expected {field}=true but got {:?}",
        json[&field]
    );
}

#[then(expr = "the worker tool result JSON should have {string} equal to false")]
fn then_wtw_json_field_false(world: &mut QuectoWorld, field: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    assert_eq!(
        json[&field], false,
        "expected {field}=false but got {:?}",
        json[&field]
    );
}

#[then(expr = "the worker tool result JSON should have a non-empty {string}")]
fn then_wtw_json_field_non_empty(world: &mut QuectoWorld, field: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let val = &json[&field];
    match val {
        serde_json::Value::String(s) => {
            assert!(!s.is_empty(), "expected non-empty '{field}'");
        }
        serde_json::Value::Null => {
            panic!("expected non-empty '{field}' but got null");
        }
        _ => {} // non-null, non-string — considered non-empty
    }
}

#[then(expr = "the worker tool result JSON should have {string} greater than {int}")]
fn then_wtw_json_field_greater(world: &mut QuectoWorld, field: String, threshold: i64) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let val = json[&field]
        .as_i64()
        .unwrap_or_else(|| panic!("'{field}' is not a number: {:?}", json[&field]));
    assert!(
        val > threshold,
        "expected {field} > {threshold} but got {val}"
    );
}

#[then(expr = "the worker tool result content should contain {string}")]
fn then_wtw_content_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.wtw_last_result.as_ref().expect("no result");
    let content = match result {
        Ok(tr) => &tr.content,
        Err(e) => e,
    };
    assert!(
        content.contains(&expected),
        "expected content to contain '{expected}' but got: {content}"
    );
}

#[then(expr = "the worker tool result JSON {string} should contain {string}")]
fn then_wtw_json_str_contains(world: &mut QuectoWorld, field: String, expected: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let val = json[&field]
        .as_str()
        .unwrap_or_else(|| panic!("'{field}' is not a string: {:?}", json[&field]));
    assert!(
        val.contains(&expected),
        "expected '{field}' to contain '{expected}' but got: {val}"
    );
}

#[then(expr = "the worker tool result JSON {string} array should not be empty")]
fn then_wtw_json_array_not_empty(world: &mut QuectoWorld, field: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let arr = json[&field]
        .as_array()
        .unwrap_or_else(|| panic!("'{field}' is not an array: {:?}", json[&field]));
    assert!(!arr.is_empty(), "expected non-empty '{field}' array");
}

#[then(expr = "the worker tool result JSON {string} array should be empty")]
fn then_wtw_json_array_empty(world: &mut QuectoWorld, field: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let arr = json[&field]
        .as_array()
        .unwrap_or_else(|| panic!("'{field}' is not an array: {:?}", json[&field]));
    assert!(
        arr.is_empty(),
        "expected empty '{field}' array but got {arr:?}"
    );
}

#[then(expr = "the worker tool result JSON {string} array should contain {string}")]
fn then_wtw_json_array_contains(world: &mut QuectoWorld, field: String, expected: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let arr = json[&field]
        .as_array()
        .unwrap_or_else(|| panic!("'{field}' is not an array: {:?}", json[&field]));
    assert!(
        arr.iter().any(|v| v.as_str() == Some(&expected)),
        "expected '{field}' array to contain '{expected}' but got {arr:?}"
    );
}

#[then(
    expr = "the worker tool result JSON {string} array should not contain a file matching {string}"
)]
fn then_wtw_json_array_no_file(world: &mut QuectoWorld, field: String, pattern: String) {
    let content = wtw_result_content(world);
    let json: serde_json::Value = serde_json::from_str(&content).expect("invalid JSON");
    let arr = json[&field]
        .as_array()
        .unwrap_or_else(|| panic!("'{field}' is not an array: {:?}", json[&field]));
    for item in arr {
        if let Some(obj) = item.as_object() {
            if let Some(file) = obj.get("file").and_then(|f| f.as_str()) {
                assert!(
                    !file.contains(&pattern),
                    "found file '{file}' matching '{pattern}' — should be gitignored"
                );
            }
        }
    }
}

#[then(expr = "the file {string} in the job directory should still contain {string}")]
fn then_file_still_contains(world: &mut QuectoWorld, path: String, expected: String) {
    let job_dir = world.wtw_job_dir.as_ref().expect("job dir not set");
    let content = std::fs::read_to_string(job_dir.join(&path))
        .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
    assert!(
        content.contains(&expected),
        "expected '{path}' to contain '{expected}' but got: {content}"
    );
}

#[then(expr = "the worker tool registry should contain exactly {int} tools")]
fn then_registry_has_n_tools(world: &mut QuectoWorld, count: usize) {
    let registry = world.wtw_registry.as_ref().expect("registry not set");
    let names = registry.names();
    assert_eq!(
        names.len(),
        count,
        "expected {count} tools but got {}: {:?}",
        names.len(),
        names
    );
}

#[then(expr = "the worker tool registry definitions should include {string}")]
fn then_registry_defs_include(world: &mut QuectoWorld, name: String) {
    let registry = world.wtw_registry.as_ref().expect("registry not set");
    let defs = registry.definitions();
    assert!(
        defs.iter().any(|d| d.name == name),
        "expected definitions to include '{name}'"
    );
}

#[then("the worker tool execution should fail with an unknown tool error")]
fn then_worker_tool_exec_fails_unknown(world: &mut QuectoWorld) {
    let result = world.wtw_last_result.as_ref().expect("no result");
    match result {
        Err(e) => {
            assert!(
                e.contains("unknown tool"),
                "expected 'unknown tool' error but got: {e}"
            );
        }
        Ok(tr) => {
            panic!(
                "expected an error but got Ok: is_error={}, content={}",
                tr.is_error, tr.content
            );
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Extract the content string from the last successful worker tool result.
fn wtw_result_content(world: &QuectoWorld) -> String {
    let result = world.wtw_last_result.as_ref().expect("no result");
    match result {
        Ok(tr) => tr.content.clone(),
        Err(e) => panic!("expected Ok result but got Err: {e}"),
    }
}
