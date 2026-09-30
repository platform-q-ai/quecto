//! #2341: a binding refusal's `tracing` record names the wrong arguments
//! by the schema's names only; a secret-shaped key never reaches it; and
//! the allowlist, the expected types and the recording rule it keeps.
use serde_json::json;

use super::super::call;
use super::super::tests::captured;
use super::{
    WHOLE, allowlisted, expected_type, faults, recorded, schema_field, unreadable_arguments,
};
use crate::domain::swarm::{ArgumentFaults, RefusalKind, UnexpectedArgs, WrongTypeArg};
use crate::infrastructure::tools::swarm_board_ops::BOARD_OPS;

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
        // The call's redacted actor ref, to match it to its call (#2346
        // review nit).
        assert!(line.contains("member=\"parent\""), "{line}");
        assert!(line.contains("quecto::swarm_board"), "{line}");
    }
}

/// Every board op's field is its own schema name; nothing else is one,
/// the harness's own fields and `op` included.
#[test]
fn only_a_board_ops_field_is_a_schema_name() {
    for arg in BOARD_OPS.iter().flat_map(|spec| spec.args.iter()) {
        assert_eq!(schema_field(arg.name), Some(arg.name));
    }
    for key in [SECRET, "op", "member_limit", "Title", "title ", "", WHOLE] {
        assert_eq!(schema_field(key), None, "{key}");
    }
    let named = |name: &str| ArgumentFaults {
        missing_args: vec![name.to_owned()],
        ..ArgumentFaults::NONE
    };
    assert!(allowlisted(&named("token")));
    assert!(allowlisted(&named(WHOLE)));
    assert!(!allowlisted(&named("owner")));
    assert!(!allowlisted(&named(SECRET)));
}

/// Each field's expected type, labelled from its schema, and what it
/// admits: an integer is no float or boolean; an array's items count.
#[test]
fn expected_types_are_labelled_from_the_schema() {
    for (name, label) in [
        ("task_id", "integer"),
        ("token", "string"),
        ("revision", "null_or_string"),
        ("supersedes", "null_or_integer"),
        ("passed", "boolean"),
        ("kind", "string"),
        ("acceptance", "array_of_string"),
        ("dependencies", "null_or_array_of_integer"),
        ("evidence", "array_of_object"),
    ] {
        assert_eq!(expected_type(name).unwrap().label, label, "{name}");
    }
    assert!(expected_type("owner").is_none());
    let task_id = expected_type("task_id").unwrap();
    assert!(task_id.admits(&json!(3)) && task_id.admits(&json!(u64::MAX)));
    for value in [json!(1.0), json!(true), json!("1"), json!(null)] {
        assert!(!task_id.admits(&value), "{value}");
    }
    let dependencies = expected_type("dependencies").unwrap();
    assert!(dependencies.admits(&json!(null)) && dependencies.admits(&json!([1, 2])));
    assert!(!dependencies.admits(&json!([1, "2"])) && !dependencies.admits(&json!(1)));
}

/// A call's faults against its signature: arguments that are no array or
/// object are `arguments`; a harness-internal method's parameter no schema
/// field names is left out, and its values are not type-checked.
#[test]
fn faults_name_schema_fields_only() {
    let release = super::Method::parse("release").unwrap();
    assert_eq!(
        faults(release, &json!("text")).unreadable_args,
        [WHOLE],
        "no array or object"
    );
    assert_eq!(
        faults(release, &json!({"task_id": "1", "token": 2})).wrong_type_args,
        [
            WrongTypeArg {
                arg: "task_id".into(),
                expected: "integer".into()
            },
            WrongTypeArg {
                arg: "token".into(),
                expected: "string".into()
            },
        ]
    );
    let admit = super::Method::parse("_admit").unwrap();
    assert_eq!(
        faults(admit, &json!({"member": 1, "reservation": 2, (SECRET): 1})),
        ArgumentFaults {
            unexpected_args: Some(UnexpectedArgs {
                count: 1,
                known: vec![],
            }),
            ..ArgumentFaults::NONE
        },
        "a harness-internal method's values are not type-checked"
    );
    let socket = super::Method::parse("_socket").unwrap();
    assert_eq!(
        faults(socket, &json!({})),
        ArgumentFaults::NONE,
        "socket is no schema field"
    );
}

/// Unreadable member text names each schema field once, and `arguments`
/// for the text as a whole or a key no schema field names.
#[test]
fn unreadable_input_names_its_fields_or_the_whole() {
    let names = |faults: ArgumentFaults| faults.unreadable_args;
    assert_eq!(names(unreadable_arguments(None)), [WHOLE]);
    assert_eq!(names(unreadable_arguments(Some(vec![]))), [WHOLE]);
    assert_eq!(
        names(unreadable_arguments(Some(vec![String::new()]))),
        [WHOLE]
    );
    assert_eq!(
        names(unreadable_arguments(Some(vec![
            "title".into(),
            "title".into(),
            SECRET.into(),
            "task_id".into(),
        ]))),
        ["title", "task_id", WHOLE]
    );
}

/// Faults are kept for a `calling` or `invalid` refusal only.
#[test]
fn faults_are_recorded_for_a_binding_refusal_only() {
    let found = || {
        Some(ArgumentFaults {
            missing_args: vec!["token".into()],
            ..ArgumentFaults::NONE
        })
    };
    for kind in [RefusalKind::Calling, RefusalKind::Invalid] {
        assert!(!recorded::<()>(&Err(kind), found()).is_empty(), "{kind:?}");
    }
    for outcome in [
        Err(RefusalKind::NotFound),
        Err(RefusalKind::Store),
        Ok("claimed"),
    ] {
        assert!(recorded(&outcome, found()).is_empty(), "{outcome:?}");
    }
    assert!(recorded::<()>(&Err(RefusalKind::Calling), None).is_empty());
}
