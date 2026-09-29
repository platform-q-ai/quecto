//! The structured ops' wire text (#2279; parent decision on #2278/#2279):
//! a board answer reaches the member as the text Python's `json.dumps`
//! writes, compared byte for byte (`run_both_wire`), where the other
//! scenarios compare values after a serde round trip that erases float
//! spelling (`1e+16` against serde's `1e16`), escapes and key order.
use serde_json::json;

use crate::swarm_board_diff_loose_runs::create_text;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_both_wire, step_text};

/// Floats Python and serde spell differently, integers at the edge of
/// u64, keys out of order, non-ASCII and separator characters, through
/// the run's criteria (the summary), a task, the task list, the event
/// page, the inbox, a control receipt and the usage report.
#[test]
fn board_answers_are_python_s_wire_text() {
    let extra = r#"{"z": 1e16, "a": [1e-07, 0.1, -0.0, 2.5e-300, 18446744073709551615, -0], "\u00e9": "\u00fc\u2028\ud83d\ude00", "k": {"y": null, "b": true}}"#;
    run_both_wire(&[
        step_text("parent", "create_run", &create_text(extra), NOW),
        at(1.0, "parent", "summary", json!([])),
        step_text(
            "parent",
            "task_create",
            r#"["r1", "na\u00efve \ud83d\ude00", ["tests \"pass\""], []]"#,
            NOW + 2.0,
        ),
        at(3.0, "parent", "claim", json!([1])),
        at(4.0, "parent", "task", json!([1])),
        at(5.0, "parent", "tasks", json!({"offset": 0, "limit": 5})),
        step_text(
            "parent",
            "send",
            r#"{"request": "m1", "recipient": "parent", "body": "caf\u00e9\n\ttab", "revision": "R\u00e9"}"#,
            NOW + 6.0,
        ),
        at(7.0, "parent", "inbox", json!([])),
        at(8.0, "parent", "events", json!([0, 25])),
        at(9.0, "parent", "stop", json!(["blocked", "wait for \u{e9}"])),
        at(10.0, "parent", "usage_report", json!([])),
        at(11.0, "parent", "task", json!([2])),
    ]);
}

/// #2279 final review: a structured op with two fields its method does not
/// take is refused naming the first the member wrote, as Python's call
/// with those keywords raises naming the first (`board.tasks(x=1, y=2)`).
#[test]
fn an_op_with_two_unexpected_fields_names_the_first_as_python_does() {
    use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
    use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;
    use quecto::application::tools::ports::Tool;
    let dir = tempfile::tempdir().unwrap();
    let python_root = dir.path().join("python");
    std::fs::create_dir_all(&python_root).unwrap();
    let mut python = PyBoard::start(&python_root.join("swarm.sqlite"), &python_root, dir.path());
    let Outcome::Raised(raised) = python.call("parent", "tasks", r#"{"x": 1, "y": 2}"#, NOW) else {
        panic!("Python raises on the unexpected keywords");
    };
    let named = raised
        .split("unexpected keyword argument '")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .unwrap_or_else(|| panic!("{raised}"))
        .to_owned();
    assert_eq!(named, "x", "{raised}");

    let checkout = dir.path().join("rust");
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = quecto::infrastructure::tools::swarm_bridge::SwarmContext {
        board: quecto::composition::swarm::swarm_board(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        checkout: checkout.clone(),
        member: "coordinator".into(),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    let pid = std::process::id();
    context
        .create_run(
            &json!({"goal": "ship", "constraints": [],
                "criteria": [{"id": "tests", "kind": "command", "description": "pass"}],
                "member_limit": 1, "deadline": deadline}),
            &quecto::domain::swarm::ProcessIdentity {
                pid,
                started: quecto::infrastructure::tools::swarm_bridge::process_start(pid).unwrap(),
            },
            None,
        )
        .unwrap();
    let tool = quecto::infrastructure::tools::swarm::SwarmTool::new()
    .with_context(Some(context));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(tool.execute(r#"{"op": "tasks", "x": 1, "y": 2}"#))
        .unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .contains(&format!("tasks: unexpected argument {named}")),
        "Python named {named}: {}",
        result.content
    );
}
