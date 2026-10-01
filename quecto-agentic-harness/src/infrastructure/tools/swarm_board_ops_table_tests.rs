//! #2279: `BOARD_OPS` is the epic's structured-op mapping table (owner
//! decision D4), binds as the dispatcher binds, and is the one source of
//! the tool schema's ops and fields.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{BOARD_OPS, MUTATING_OPS, READ_ONLY_OPS, tool_schema};
use crate::infrastructure::tools::swarm_board_dispatch;

/// A Python signature: each parameter's name and its default (`None` for
/// a required one).
type Signature = Vec<(&'static str, Option<Value>)>;

/// The epic's mapping table: each `board.` method's name and Python
/// signature.
fn mapping_table() -> Vec<(&'static str, Signature)> {
    let required = |name| (name, None);
    vec![
        ("task", vec![required("task_id")]),
        (
            "tasks",
            vec![("offset", Some(json!(0))), ("limit", Some(json!(50)))],
        ),
        (
            "file_owners",
            vec![("offset", Some(json!(0))), ("limit", Some(json!(50)))],
        ),
        (
            "task_create",
            vec![
                required("request"),
                required("title"),
                required("acceptance"),
                ("dependencies", Some(Value::Null)),
            ],
        ),
        (
            "dependencies",
            vec![required("task_id"), required("dependencies")],
        ),
        ("claim", vec![required("task_id")]),
        ("release", vec![required("task_id"), required("token")]),
        (
            "block",
            vec![required("task_id"), required("token"), required("reason")],
        ),
        (
            "unblock",
            vec![required("task_id"), required("token"), required("reason")],
        ),
        (
            "submit",
            vec![required("task_id"), required("token"), required("evidence")],
        ),
        (
            "reserve",
            vec![required("task_id"), required("token"), required("paths")],
        ),
        (
            "release_files",
            vec![
                required("task_id"),
                required("token"),
                required("reservation"),
            ],
        ),
        (
            "send",
            vec![
                required("request"),
                required("recipient"),
                required("body"),
                ("revision", Some(Value::Null)),
                ("supersedes", Some(Value::Null)),
            ],
        ),
        ("withdraw", vec![required("message_id")]),
        ("inbox", vec![("include_consumed", Some(json!(false)))]),
        ("ack", vec![required("message_id")]),
        (
            "evidence",
            vec![
                required("criterion"),
                required("artifact"),
                required("revision"),
                required("kind"),
                required("passed"),
            ],
        ),
        (
            "amend",
            vec![
                required("goal"),
                required("constraints"),
                required("criteria"),
                required("reason"),
            ],
        ),
        (
            "verify_task",
            vec![required("task_id"), required("token"), required("revision")],
        ),
        (
            "revalidate_task",
            vec![
                required("task_id"),
                required("revision"),
                required("evidence"),
            ],
        ),
        (
            "recover",
            vec![required("task_id"), ("release_files", Some(json!(false)))],
        ),
        ("revoke", vec![required("task_id"), required("reason")]),
        ("complete", vec![required("revision")]),
        ("stop", vec![required("status"), required("reason")]),
        ("usage_report", vec![]),
    ]
}

fn spec_signature(spec: &super::OpSpec) -> Signature {
    spec.args
        .iter()
        .map(|arg| {
            assert_eq!(
                arg.required,
                arg.default.is_none(),
                "{}.{}",
                spec.name,
                arg.name
            );
            let default = arg
                .default
                .map(|text| serde_json::from_str(text).expect("a default is JSON text"));
            (arg.name, default)
        })
        .collect()
}

#[test]
fn board_ops_are_the_mapping_table() {
    let table = mapping_table();
    let names: Vec<&str> = BOARD_OPS.iter().map(|spec| spec.name).collect();
    let expected: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, expected);
    for (spec, (_, signature)) in BOARD_OPS.iter().zip(&table) {
        assert_eq!(&spec_signature(spec), signature, "{}", spec.name);
    }
}

/// The table binds as the dispatcher binds: the same parameters in the
/// same order with the same defaults.
#[test]
fn board_ops_bind_as_the_dispatcher_binds() {
    assert_eq!(BOARD_OPS.len(), 25, "every op of the mapping table");
    for spec in BOARD_OPS {
        let signature = swarm_board_dispatch::signature(spec.name)
            .unwrap_or_else(|| panic!("the dispatcher serves {}", spec.name));
        assert_eq!(signature, spec_signature(spec), "{}", spec.name);
    }
}

#[test]
fn read_only_and_mutating_ops_partition_the_board_ops() {
    let read: BTreeSet<&str> = READ_ONLY_OPS.iter().copied().collect();
    let mutating: BTreeSet<&str> = MUTATING_OPS.iter().copied().collect();
    let all: BTreeSet<&str> = BOARD_OPS.iter().map(|spec| spec.name).collect();
    assert_eq!(read.len(), READ_ONLY_OPS.len(), "no op is listed twice");
    assert_eq!(mutating.len(), MUTATING_OPS.len(), "no op is listed twice");
    assert!(read.is_disjoint(&mutating));
    assert_eq!(&read | &mutating, all);
    assert_eq!(
        read,
        ["file_owners", "inbox", "task", "tasks", "usage_report"]
            .into_iter()
            .collect()
    );
    for spec in BOARD_OPS {
        assert_eq!(spec.read_only, read.contains(spec.name), "{}", spec.name);
    }
}

/// Every argument whose default is `null` admits `null` in the schema
/// (#2279 review N8).
#[test]
fn a_null_default_is_an_allowed_value() {
    for spec in BOARD_OPS {
        for arg in spec.args.iter().filter(|arg| arg.default == Some("null")) {
            let schema: Value = serde_json::from_str(arg.json_type).unwrap();
            let types = match &schema["type"] {
                Value::Array(types) => types.clone(),
                single => vec![single.clone()],
            };
            assert!(
                types.contains(&json!("null")),
                "{}.{}: {schema}",
                spec.name,
                arg.name
            );
        }
    }
}

#[test]
fn the_checked_in_schema_is_the_rendering_of_board_ops() {
    let checked_in: Value =
        serde_json::from_str(include_str!("swarm_assets/tool_schema.json")).unwrap();
    assert_eq!(checked_in, tool_schema());
    let ops: Vec<&str> = checked_in["properties"]["op"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    for spec in BOARD_OPS {
        assert!(ops.contains(&spec.name), "{}", spec.name);
        for arg in spec.args {
            let rendered: Value = serde_json::from_str(arg.json_type).unwrap();
            assert_eq!(
                checked_in["properties"][arg.name], rendered,
                "{}.{}",
                spec.name, arg.name
            );
        }
    }
}

/// The fields the issue lists, typed as it lists them.
#[test]
fn the_schema_types_the_new_fields() {
    let schema = tool_schema();
    let property = |name: &str| schema["properties"][name].clone();
    for name in [
        "token",
        "reason",
        "reservation",
        "request",
        "recipient",
        "body",
        "criterion",
        "artifact",
        "title",
        "status",
    ] {
        assert_eq!(property(name), json!({"type": "string"}), "{name}");
    }
    for name in ["task_id", "message_id"] {
        assert_eq!(property(name), json!({"type": "integer"}), "{name}");
    }
    // #2279 review N8: a field some op defaults to `null` admits `null`
    // (one schema per field, so for every op that names it).
    assert_eq!(property("revision"), json!({"type": ["string", "null"]}));
    assert_eq!(property("supersedes"), json!({"type": ["integer", "null"]}));
    for name in ["include_consumed", "passed", "release_files"] {
        assert_eq!(property(name), json!({"type": "boolean"}), "{name}");
    }
    for name in ["paths", "acceptance"] {
        assert_eq!(
            property(name),
            json!({"type": "array", "items": {"type": "string"}}),
            "{name}"
        );
    }
    assert_eq!(
        property("dependencies"),
        json!({"type": ["array", "null"], "items": {"type": "integer"}})
    );
    assert_eq!(
        property("kind"),
        json!({"type": "string", "enum": ["command", "review"]})
    );
    assert_eq!(
        property("evidence"),
        json!({"type": "array", "items": {"type": "object",
            "properties": {"artifact": {"type": "string"}, "revision": {"type": "string"}},
            "required": ["artifact", "revision"]}})
    );
    assert_eq!(schema["required"], json!(["op"]));
}

#[test]
fn the_description_teaches_the_structured_ops_not_python() {
    let description = include_str!("swarm_assets/tool_description.txt");
    for spec in BOARD_OPS {
        let args: Vec<&str> = spec.args.iter().map(|arg| arg.name).collect();
        let call = format!("{}({})", spec.name, args.join(","));
        assert!(description.contains(&call), "{call} is not taught");
    }
    for workbench in ["from swarm import board", "board.", "RLIMIT", "op=run"] {
        assert!(
            !description.contains(workbench),
            "{workbench} is still taught"
        );
    }
    assert!(
        description.contains(r#"{"op":"claim","task_id":"#),
        "an example of a structured op"
    );
}

/// #2394: no member-facing op answers a bare `null` any more (a task op
/// answers the task's row, `evidence` the recorded evidence), so the
/// description no longer says ops answer "null for none".
#[test]
fn the_description_promises_no_null_answers() {
    let description = include_str!("swarm_assets/tool_description.txt");
    assert!(
        !description.contains("null for none"),
        "the description still says ops answer null"
    );
}
