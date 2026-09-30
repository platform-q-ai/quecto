//! #2341: a binding refusal's `tracing` record names the wrong arguments
//! by the schema's names only; a secret-shaped key never reaches it.
use serde_json::json;

use super::super::call;
use super::captured;

const SECRET: &str = "sk-ant-api03-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
const MARKER: &str = "swarm board call arguments";

#[test]
fn binding_faults_are_traced_by_schema_name_only() {
    let log = captured(4, |handles| {
        call(handles, "parent", "block", json!({"task_id": 1})).unwrap_err();
        call(
            handles,
            "parent",
            "claim",
            json!({"task_id": 1, (SECRET): "x", "title": SECRET}),
        )
        .unwrap_err();
        call(handles, "parent", "release", json!([1, "t", SECRET])).unwrap_err();
        // A refusal no binding raised leaves no such record.
        let _refused = call(handles, "parent", "_snapshot", json!([]));
    });
    assert!(!log.contains(SECRET), "{log}");
    let lines: Vec<&str> = log.lines().filter(|line| line.contains(MARKER)).collect();
    assert_eq!(lines.len(), 3, "{log}");
    for (line, fields) in lines.iter().zip([
        &["op=\"block\"", "missing_args=token,reason "][..],
        &[
            "op=\"claim\"",
            "unexpected_args=2",
            "unexpected_known=title",
        ][..],
        &[
            "op=\"release\"",
            "unexpected_args=1",
            "unexpected_known= wrong_type_args=",
        ][..],
    ]) {
        for field in fields {
            assert!(line.contains(field), "{field} in {line}");
        }
        assert!(line.contains("quecto::swarm_board"), "{line}");
    }
}
