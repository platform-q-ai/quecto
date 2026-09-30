use super::*;

#[test]
fn an_op_refused_during_setup_names_op_create_with_a_relative_deadline() {
    let message = op_refused("claim", Some("setup"));
    assert!(message.contains(r#""op":"create""#), "{message}");
    assert!(
        message.contains("deadline_in_seconds"),
        "no stale absolute deadline: {message}"
    );
    assert!(message.contains("Allowed now: create"), "{message}");
    assert!(
        !message.contains("\"setup\""),
        "the status is not quoted: {message}"
    );
}

#[test]
fn an_op_refused_when_paused_names_every_allowed_op_and_the_supervisor() {
    let message = op_refused("claim", Some("paused"));
    for op in ["summary", "events", "usage", "reconcile"] {
        assert!(message.contains(op), "{op}: {message}");
    }
    // A worker told to cancel_run would only be refused: it is coordinator-only.
    assert!(
        message.contains("the coordinator may also usage_budget and cancel_run"),
        "{message}"
    );
    assert!(message.contains("supervisor"), "{message}");
}

#[test]
fn an_ended_or_unreadable_run_says_what_to_do() {
    let ended = op_refused("claim", Some("succeeded"));
    assert!(
        ended.contains("succeeded") && ended.contains("Allowed: summary"),
        "{ended}"
    );
    assert!(op_refused("claim", None).contains("op=summary"));
    assert!(op_deadline_passed("claim").contains("extend"));
}

#[test]
fn an_unknown_op_lists_the_valid_ones_and_how_to_end_a_run() {
    let message = unknown_op("stop");
    for op in VALID_OPS {
        assert!(message.contains(op), "{op}: {message}");
    }
    assert!(
        message
            .contains("the coordinator calls op=stop (status, reason) or op=complete (revision)"),
        "{message}"
    );
    assert!(!message.contains("board."), "no Python call: {message}");
}

/// #2279: every structured board op is a valid op, beside the harness's
/// own.
#[test]
fn the_valid_ops_are_the_harness_ops_and_the_board_ops() {
    let mut expected: Vec<&str> = [
        "create",
        "summary",
        "reconcile",
        "cancel_run",
        "pause",
        "resume",
        "events",
        "usage",
        "usage_budget",
    ]
    .into_iter()
    .chain(
        crate::infrastructure::tools::swarm_board_ops::BOARD_OPS
            .iter()
            .map(|spec| spec.name),
    )
    .collect();
    assert_eq!(expected.len(), 9 + 25, "the harness's ops and the table's");
    let mut ops = VALID_OPS.to_vec();
    expected.sort_unstable();
    ops.sort_unstable();
    assert_eq!(ops, expected);
}

/// #2279: a structured op refused by the running gate names itself, and
/// what is allowed instead.
#[test]
fn a_gated_board_op_names_itself_and_what_is_allowed() {
    let paused = op_refused("claim", Some("paused"));
    assert!(
        paused.starts_with("the swarm run is paused, so op=claim is unavailable. Allowed:"),
        "{paused}"
    );
    let setup = op_refused("inbox", Some("setup"));
    assert!(
        setup.contains("(status setup), so op=inbox is unavailable"),
        "{setup}"
    );
    assert!(setup.contains(r#""op":"create""#), "{setup}");
    let ended = op_refused("tasks", Some("succeeded"));
    assert!(
        ended.contains("the swarm run is succeeded, so op=tasks is unavailable"),
        "{ended}"
    );
    let late = op_deadline_passed("send");
    assert!(late.contains("so op=send is unavailable"), "{late}");
}

#[test]
fn the_valid_ops_are_exactly_the_schema_enum() {
    // A drift guard: an op added to the schema (or the list) without the
    // other would make the unknown-op guidance lie.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("swarm_assets/tool_schema.json")).unwrap();
    let mut schema_ops: Vec<&str> = schema["properties"]["op"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    let mut ops = VALID_OPS.to_vec();
    schema_ops.sort_unstable();
    ops.sort_unstable();
    assert_eq!(ops, schema_ops);
}

#[test]
fn the_schema_lists_constraints_as_an_optional_list_of_strings() {
    // #2205: the store defaults an omitted list to empty, so the schema
    // must not require it.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("swarm_assets/tool_schema.json")).unwrap();
    assert_eq!(schema["required"], serde_json::json!(["op"]));
    assert_eq!(
        schema["properties"]["constraints"],
        serde_json::json!({"type": "array", "items": {"type": "string"}})
    );
}

#[test]
fn the_tool_description_names_op_create_and_a_relative_deadline() {
    let description = include_str!("swarm_assets/tool_description.txt");
    assert!(description.contains("op=create"), "no op=create");
    assert!(description.contains("deadline_in_seconds"));
}

#[tokio::test]
async fn a_board_op_before_create_points_the_founder_at_op_create() {
    // End to end through the tool: a bootstrapped, not yet created run.
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().to_path_buf();
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let context = crate::infrastructure::tools::swarm_bridge::SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(crate::application::ports::SwarmTestLifecycle),
        checkout: workspace,
        member: "coordinator".into(),
    };
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.call(
            "_bootstrap",
            serde_json::json!([1, "start", "/tmp/unused.sock"]),
        )
    })
    .unwrap();
    let tool = super::super::SwarmTool::new().with_context(Some(context));
    let request = r#"{"op":"task_create","request":"r1","title":"t","acceptance":["pass"]}"#;
    let result = tool.execute(request).await.unwrap();
    assert!(result.is_error, "{request}: {}", result.content);
    assert!(
        result.content.contains(r#""op":"create""#),
        "{request}: {}",
        result.content
    );
}

/// #2279 review N9: every refusal that allows `usage` says it is the
/// harness op, not the board op `usage_report`, which the running gate
/// refuses with the rest.
#[test]
fn a_refusal_that_allows_usage_says_usage_report_is_not_it() {
    for message in [
        op_refused("usage_report", Some("paused")),
        op_refused("usage_report", Some("setup")),
        op_refused("usage_report", Some("succeeded")),
        op_deadline_passed("usage_report"),
    ] {
        assert!(
            message.contains("usage (op=usage; the board op usage_report needs a running run)"),
            "{message}"
        );
    }
}

/// The swarm tool over a created, running run, as the tests above build it.
fn running_tool(directory: &tempfile::TempDir) -> super::super::SwarmTool {
    super::super::super::swarm_test_support::tool(
        std::sync::Arc::new(directory.path().to_path_buf()),
        crate::composition::swarm::swarm_board(),
    )
}

/// #2282: the Python workbench ops are gone. Each is an unknown op whose
/// refusal lists the valid ops, the structured board ops among them, and
/// never runs anything.
#[tokio::test]
async fn removed_ops_are_unknown_and_name_the_structured_alternative() {
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let tool = running_tool(&directory);
    for (op, request) in [
        ("run", r#"{"op":"run","code":"open('ran','w').write('x')"}"#),
        ("status", r#"{"op":"status","job_id":"job_1"}"#),
        ("output", r#"{"op":"output","job_id":"job_1"}"#),
        ("cancel", r#"{"op":"cancel","job_id":"job_1"}"#),
    ] {
        let result = tool.execute(request).await.unwrap();
        assert!(result.is_error, "{request}: {}", result.content);
        let prefix = format!("unknown op {op}; valid ops: ");
        assert!(
            result.content.contains(&prefix),
            "{request}: {}",
            result.content
        );
        assert!(result.content.contains("claim"), "{}", result.content);
        assert!(!directory.path().join("ran").exists(), "{request} ran code");
    }
    for removed in ["run", "status", "output", "cancel"] {
        assert!(!VALID_OPS.contains(&removed), "{removed} is still valid");
    }
}

/// #2282: a call without an op no longer defaults to running Python; it is
/// refused with the valid ops, and nothing runs.
#[tokio::test]
async fn a_call_without_an_op_is_refused_with_the_valid_ops() {
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let tool = running_tool(&directory);
    for request in [r#"{"code":"open('ran','w').write('x')"}"#, "{}"] {
        let result = tool.execute(request).await.unwrap();
        assert!(result.is_error, "{request}: {}", result.content);
        assert!(
            result
                .content
                .contains("op is required; valid ops: create, "),
            "{request}: {}",
            result.content
        );
        assert!(!directory.path().join("ran").exists(), "{request} ran code");
    }
}

/// #2282 review N1: an op that is present but not a string (a number,
/// `null`) is not a missing op: the refusal says the op must be a string
/// naming a valid op, and lists them.
#[tokio::test]
async fn an_op_that_is_not_a_string_is_refused_as_such() {
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let tool = running_tool(&directory);
    for request in [
        r#"{"op":3,"code":"open('ran','w').write('x')"}"#,
        r#"{"op":null}"#,
        r#"{"op":["claim"]}"#,
    ] {
        let result = tool.execute(request).await.unwrap();
        assert!(result.is_error, "{request}: {}", result.content);
        assert!(
            result
                .content
                .contains("op must be a string naming one of: create, "),
            "{request}: {}",
            result.content
        );
        assert!(
            !result.content.contains("op is required"),
            "{request}: {}",
            result.content
        );
        assert!(!directory.path().join("ran").exists(), "{request} ran code");
    }
}

/// #2282 review L3: arguments that are not JSON are refused as such, and
/// nothing runs.
#[tokio::test]
async fn arguments_that_are_not_json_are_refused() {
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let tool = running_tool(&directory);
    let result = tool.execute("not-json").await.unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.contains("invalid JSON arguments"),
        "{}",
        result.content
    );
}

#[test]
fn an_op_not_a_string_lists_every_valid_op() {
    let message = op_not_a_string();
    assert!(
        message.starts_with("op must be a string naming one of: "),
        "{message}"
    );
    for op in VALID_OPS {
        assert!(message.contains(op), "{op}: {message}");
    }
}

/// #2282: the schema carries no execution argument.
#[test]
fn the_schema_has_no_execution_arguments() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("swarm_assets/tool_schema.json")).unwrap();
    for removed in [
        "code",
        "path",
        "args",
        "stdin",
        "timeout_seconds",
        "max_output_bytes",
        "background",
        "job_id",
    ] {
        assert!(
            schema["properties"].get(removed).is_none(),
            "the schema still offers {removed}"
        );
    }
}
