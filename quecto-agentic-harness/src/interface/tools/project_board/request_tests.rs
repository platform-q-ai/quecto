use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn refused(value: Value) -> RequestError {
    parse(&value).expect_err("refused")
}

#[test]
fn each_op_reads_its_own_fields() {
    let task = || slug("a");
    let cases = [
        (json!({"op": "bootstrap"}), BoardRequest::Bootstrap),
        (json!({"op": "list"}), BoardRequest::List(ListFilter::All)),
        (
            json!({"op": "list", "status": "review"}),
            BoardRequest::List(ListFilter::Status(TaskStatus::Review)),
        ),
        (
            json!({"op": "list", "parent": "e"}),
            BoardRequest::List(ListFilter::Children(slug("e"))),
        ),
        (
            json!({"op": "list", "dependents_of": "d"}),
            BoardRequest::List(ListFilter::Dependents(slug("d"))),
        ),
        (json!({"op": "list_ready"}), BoardRequest::ListReady),
        (json!({"op": "list_takeable"}), BoardRequest::ListTakeable),
        (
            json!({"op": "show", "task": "a"}),
            BoardRequest::Show(task()),
        ),
        (
            json!({"op": "move", "task": "a", "to": "in_progress"}),
            BoardRequest::Change(task(), BoardOp::Move(TaskStatus::InProgress)),
        ),
        (
            json!({"op": "renew", "task": "a"}),
            BoardRequest::Change(task(), BoardOp::Renew),
        ),
        (
            json!({"op": "take_over", "task": "a"}),
            BoardRequest::Change(task(), BoardOp::TakeOver),
        ),
        (
            json!({"op": "link_pr", "task": "a", "pr": 2493}),
            BoardRequest::Change(task(), BoardOp::AddPr(2493)),
        ),
        (
            json!({"op": "unlink_pr", "task": "a", "pr": 7}),
            BoardRequest::Change(task(), BoardOp::RemovePr(7)),
        ),
        (
            json!({"op": "complete", "task": "a"}),
            BoardRequest::Complete(task()),
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40)}),
            BoardRequest::Clear {
                path: "tasks/x.json".into(),
                object: "a".repeat(40),
                repeated: BTreeMap::new(),
            },
        ),
        (
            json!({
                "op": "clear_set_aside",
                "path": "tasks/x.json",
                "object": "a".repeat(40),
                "repeated": {
                    "tasks/y.json": {"keep": "b".repeat(40), "seen": ["b".repeat(40), "c".repeat(40)]},
                    "tasks/z.json": {"drop": ["d".repeat(40)]},
                },
            }),
            BoardRequest::Clear {
                path: "tasks/x.json".into(),
                object: "a".repeat(40),
                repeated: BTreeMap::from([
                    (
                        "tasks/y.json".to_string(),
                        KeepCopy::Object {
                            keep: "b".repeat(40),
                            seen: BTreeSet::from(["b".repeat(40), "c".repeat(40)]),
                        },
                    ),
                    (
                        "tasks/z.json".to_string(),
                        KeepCopy::Drop(BTreeSet::from(["d".repeat(40)])),
                    ),
                ]),
            },
        ),
    ];
    for (value, request) in cases {
        assert_eq!(parse(&value), Ok(request), "{value}");
    }
    let every: Vec<&str> = OPS.iter().map(|(op, _)| *op).collect();
    assert_eq!(
        every,
        [
            "bootstrap",
            "list",
            "list_ready",
            "list_takeable",
            "show",
            "create",
            "update_draft",
            "move",
            "renew",
            "take_over",
            "link_pr",
            "unlink_pr",
            "complete",
            "clear_set_aside"
        ],
        "no op is named plain ready; status changes are moves"
    );
}

#[test]
fn create_takes_the_tasks_own_fields_and_defaults_to_a_draft_task() {
    let created = parse(&json!({
        "op": "create",
        "task": "a",
        "title": "Board CLI",
        "description": "Why.\r\nHow.\r\n",
        "depends_on": ["b"],
    }));
    let Ok(BoardRequest::Create(new)) = created else {
        panic!("{created:?}")
    };
    assert_eq!(
        new.description, "Why.\r\nHow.\r\n",
        "taken as sent: the use case folds it"
    );
    assert_eq!(new.status, NewBoardTaskStatus::Draft);
    assert_eq!(new.kind, TaskKind::Task);
    assert_eq!(new.depends_on, vec![slug("b")]);
    // A plan is its own artifact (#2481): neither op takes one.
    for op in ["create", "update_draft"] {
        let with_plan = json!({"op": op, "task": "a", "title": "T", "plan": []});
        assert_eq!(refused(with_plan).field, "plan", "{op}");
    }
}

#[test]
fn update_draft_sets_only_what_it_names_and_null_takes_the_parent_away() {
    let edit = parse(&json!({
        "op": "update_draft",
        "task": "a",
        "parent": null,
        "title": "T",
        "expected_updated": "2026-10-10T08:00:00Z",
    }));
    let Ok(BoardRequest::UpdateDraft(update)) = edit else {
        panic!("{edit:?}")
    };
    assert_eq!(
        update,
        DraftUpdate {
            id: slug("a"),
            title: Some("T".into()),
            description: None,
            parent: Some(None),
            depends_on: None,
            expected_updated: Some(Timestamp::parse("2026-10-10T08:00:00Z").unwrap()),
        }
    );
    assert_eq!(
        refused(json!({"op": "update_draft", "task": "a"})).field,
        "op"
    );
    assert_eq!(
        refused(json!({"op": "update_draft", "task": "a", "title": "T", "expected_updated": "yesterday"})).field,
        "expected_updated"
    );
}

#[test]
fn anything_but_the_ops_own_fields_of_their_own_types_is_refused() {
    for (value, field) in [
        (json!([]), "op"),
        (json!({}), "op"),
        (json!({"op": "delete"}), "op"),
        (json!({"op": "claim"}), "op"),
        (json!({"op": "ready"}), "op"),
        (json!({"op": "renew"}), "task"),
        (
            json!({"op": "renew", "task": "a", "holder": "Ada"}),
            "holder",
        ),
        (json!({"op": "renew", "task": "Not A Slug"}), "task"),
        (json!({"op": "renew", "task": 7}), "task"),
        (json!({"op": "move", "task": "a", "to": "finished"}), "to"),
        (json!({"op": "link_pr", "task": "a", "pr": 0}), "pr"),
        (json!({"op": "link_pr", "task": "a", "pr": "12"}), "pr"),
        (json!({"op": "link_pr", "task": "a", "pr": -3}), "pr"),
        (json!({"op": "create", "task": "a"}), "title"),
        (
            json!({"op": "create", "task": "a", "title": "T", "status": "claimed"}),
            "status",
        ),
        (
            json!({"op": "create", "task": "a", "title": "T", "kind": "story"}),
            "kind",
        ),
        (
            json!({"op": "create", "task": "a", "title": "T", "depends_on": ["b", "B"]}),
            "depends_on/1",
        ),
        (json!({"op": "create", "task": "a", "title": 3}), "title"),
        (
            json!({"op": "list", "status": "ready", "parent": "e"}),
            "parent",
        ),
        (
            json!({"op": "list", "parent": "e", "dependents_of": "d"}),
            "dependents_of",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "HEAD"}),
            "object",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json"}),
            "object",
        ),
        (
            json!({"op": "clear_set_aside", "path": "", "object": "a".repeat(40)}),
            "path",
        ),
        (
            json!({"op": "clear_set_aside", "path": "x".repeat(9 * 1024 + 1), "object": "a".repeat(40)}),
            "path",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40),
                   "repeated": {"tasks/y.json": {"keep": "b".repeat(40)}}}),
            "repeated/tasks/y.json/",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40),
                   "repeated": {"tasks/y.json": {"drop": []}}}),
            "repeated/tasks/y.json/drop",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40),
                   "repeated": {"tasks/y.json": {"drop": ["b".repeat(40), "b".repeat(40)]}}}),
            "repeated/tasks/y.json/drop/1",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40),
                   "repeated": {"tasks/y.json": {"keep": "HEAD", "seen": ["b".repeat(40)]}}}),
            "repeated/tasks/y.json/keep",
        ),
        (
            // Clearing the copy it keeps: a contradiction, refused.
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "object": "a".repeat(40),
                   "repeated": {"tasks/x.json": {"keep": "a".repeat(40), "seen": ["a".repeat(40), "b".repeat(40)]}}}),
            "repeated/tasks/x.json/keep",
        ),
    ] {
        assert_eq!(refused(value.clone()).field, field, "{value}");
    }
}

/// Two valid values of a field's kind, which read differently, and one of
/// the wrong type.
fn samples(kind: Kind) -> (Value, Value, Value) {
    match kind {
        Kind::Slug => (json!("a"), json!("b"), json!(7)),
        Kind::OptionalSlug => (json!("a"), Value::Null, json!(7)),
        Kind::Slugs => (json!(["a"]), json!(["b"]), json!("a")),
        Kind::Text => (json!("x"), json!("y"), json!(7)),
        Kind::Timestamp => (
            json!("2026-10-10T08:00:00Z"),
            json!("2026-10-10T09:00:00Z"),
            json!("yesterday"),
        ),
        Kind::Status => (json!("ready"), json!("review"), json!("nowhere")),
        Kind::NewStatus => (json!("draft"), json!("ready"), json!("claimed")),
        Kind::TaskKind => (json!("epic"), json!("chore"), json!("story")),
        Kind::Pr => (json!(1), json!(2), json!(0)),
        Kind::Path => (json!("tasks/x.json"), json!("tasks/y.json"), json!(7)),
        Kind::ObjectId => (json!("a".repeat(40)), json!("b".repeat(40)), json!("HEAD")),
        Kind::Repeated => (
            json!({"tasks/a.json": {"drop": ["a".repeat(40)]}}),
            json!({"tasks/b.json": {"drop": ["a".repeat(40)]}}),
            json!("tasks/a.json"),
        ),
    }
}

/// The table and the reader cannot drift: for every op and every field
/// the table gives it, a value of the wrong type is refused naming that
/// field, and two valid values read as two different requests.
#[test]
fn every_field_in_the_table_is_read_by_its_kind() {
    for (op, fields) in OPS {
        let base = |except: &str| {
            let mut object = serde_json::Map::new();
            object.insert("op".into(), json!(op));
            for field in fields.iter().filter(|field| field.required) {
                object.insert(field.name.into(), samples(field.kind).0);
            }
            // An edit must set something besides the field under test.
            if *op == "update_draft" {
                let other = if except == "description" {
                    "title"
                } else {
                    "description"
                };
                object.insert(other.into(), json!("D"));
            }
            object
        };
        for field in *fields {
            let (one, two, wrong) = samples(field.kind);
            let with = |value: Value| {
                let mut object = base(field.name);
                object.insert(field.name.into(), value);
                Value::Object(object)
            };
            assert_eq!(
                refused(with(wrong)).field,
                field.name,
                "{op}.{} of the wrong type",
                field.name
            );
            let (first, second) = (parse(&with(one)), parse(&with(two)));
            assert!(first.is_ok(), "{op}.{}: {first:?}", field.name);
            assert_ne!(first, second, "{op}.{} is read", field.name);
        }
    }
}
