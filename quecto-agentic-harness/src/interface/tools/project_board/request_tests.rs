use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// A show's view of its accepted plan, paged from these offsets.
fn paged(approach_from: Option<usize>, items_from: Option<usize>) -> PlanView {
    PlanView {
        version: None,
        page: PlanPage::of(approach_from, items_from),
    }
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
            BoardRequest::Show(task(), PlanView::default()),
        ),
        (
            json!({"op": "show", "task": "a", "approach_from": 0}),
            BoardRequest::Show(task(), paged(Some(0), None)),
        ),
        (
            json!({"op": "show", "task": "a", "items_from": 3, "approach_from": 900}),
            BoardRequest::Show(task(), paged(Some(900), Some(3))),
        ),
        (
            json!({"op": "show", "task": "a", "approach_from": null, "items_from": null}),
            BoardRequest::Show(task(), PlanView::default()),
        ),
        (
            json!({"op": "show", "task": "a", "approach_from": null, "items_from": 2}),
            BoardRequest::Show(task(), paged(None, Some(2))),
        ),
        (
            json!({"op": "show", "task": "a", "version": 2, "items_from": 1}),
            BoardRequest::Show(
                task(),
                PlanView {
                    version: Some(2),
                    page: PlanPage::of(None, Some(1)),
                },
            ),
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
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "a".repeat(40)}),
            BoardRequest::Clear {
                path: "tasks/x.md".into(),
                object: "a".repeat(40),
                repeated: BTreeMap::new(),
            },
        ),
        (
            json!({
                "op": "clear_set_aside",
                "path": "tasks/x.md",
                "object": "a".repeat(40),
                "repeated": {
                    "tasks/y.md": {"keep": "b".repeat(40), "seen": ["b".repeat(40), "c".repeat(40)]},
                    "tasks/z.md": {"drop": ["d".repeat(40)]},
                },
            }),
            BoardRequest::Clear {
                path: "tasks/x.md".into(),
                object: "a".repeat(40),
                repeated: BTreeMap::from([
                    (
                        "tasks/y.md".to_string(),
                        KeepCopy::Object {
                            keep: "b".repeat(40),
                            seen: BTreeSet::from(["b".repeat(40), "c".repeat(40)]),
                        },
                    ),
                    (
                        "tasks/z.md".to_string(),
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
            "propose_plan",
            "edit_plan",
            "accept_plan",
            "discard_plan",
            "start",
            "end_run",
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
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "HEAD"}),
            "object",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.md"}),
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
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "a".repeat(40),
                   "repeated": {"tasks/y.md": {"keep": "b".repeat(40)}}}),
            "repeated/tasks/y.md/",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "a".repeat(40),
                   "repeated": {"tasks/y.md": {"drop": []}}}),
            "repeated/tasks/y.md/drop",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "a".repeat(40),
                   "repeated": {"tasks/y.md": {"drop": ["b".repeat(40), "b".repeat(40)]}}}),
            "repeated/tasks/y.md/drop/1",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.md", "object": "a".repeat(40),
                   "repeated": {"tasks/y.md": {"keep": "HEAD", "seen": ["b".repeat(40)]}}}),
            "repeated/tasks/y.md/keep",
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
        Kind::Offset => (json!(0), json!(5), json!(-1)),
        Kind::Path => (json!("tasks/x.md"), json!("tasks/y.md"), json!(7)),
        Kind::ObjectId => (json!("a".repeat(40)), json!("b".repeat(40)), json!("HEAD")),
        Kind::Repeated => (
            json!({"tasks/a.md": {"drop": ["a".repeat(40)]}}),
            json!({"tasks/b.md": {"drop": ["a".repeat(40)]}}),
            json!("tasks/a.md"),
        ),
        Kind::Version => (json!(1), json!(2), json!(0)),
        Kind::AcceptedVersion => (json!(1), Value::Null, json!("1")),
        Kind::Run => (json!(1), json!(2), json!(0)),
        Kind::RunEnd => (json!("failed"), json!("lost"), json!("completed")),
        Kind::PlanItems => (
            json!([{"id": "a", "title": "A"}]),
            json!([{"id": "b", "title": "B"}]),
            json!("a"),
        ),
        Kind::Team => (
            json!({"roles": [{"name": "w", "model": "m", "effort": "low", "members": 1}]}),
            Value::Null,
            json!("w"),
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
            if *op == "edit_plan" {
                let (other, value) = match except {
                    "approach" => ("items", json!([])),
                    _ => ("approach", json!("D")),
                };
                object.insert(other.into(), value);
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

/// A value of `kind` every op accepts.
fn sample(kind: Kind) -> Value {
    match kind {
        Kind::Slug => json!("a"),
        Kind::OptionalSlug => Value::Null,
        Kind::Slugs => json!([]),
        Kind::Text => json!("t"),
        Kind::Timestamp => json!("2026-10-10T08:00:00Z"),
        Kind::Status => json!("review"),
        Kind::NewStatus => json!("draft"),
        Kind::TaskKind => json!("task"),
        Kind::Pr => json!(1),
        Kind::Offset => json!(0),
        Kind::Path => json!("p"),
        Kind::ObjectId => json!("0".repeat(40)),
        Kind::Repeated => json!({}),
        Kind::Version => json!(1),
        Kind::AcceptedVersion => Value::Null,
        Kind::Run => json!(1),
        Kind::RunEnd => json!("failed"),
        Kind::PlanItems => json!([]),
        Kind::Team => Value::Null,
    }
}

/// Epic #2481 T1: every op of `OPS`, read from its least valid request
/// (its required fields, then one optional field at a time until it
/// reads), records the op kind of its own name.
#[test]
fn every_op_records_its_own_op_kind() {
    use crate::application::project_board::telemetry::op_kind;
    for (name, fields) in OPS {
        let required = fields.iter().filter(|field| field.required);
        let mut request = json!({"op": name});
        for field in required {
            request[field.name] = sample(field.kind);
        }
        let optional = fields.iter().filter(|field| !field.required);
        let candidates = std::iter::once(request.clone()).chain(optional.map(|field| {
            let mut with = request.clone();
            with[field.name] = sample(field.kind);
            with
        }));
        let read = candidates
            .filter_map(|candidate| parse(&candidate).ok())
            .next()
            .unwrap_or_else(|| panic!("{name} reads from no minimal request"));
        assert_eq!(op_kind(&read).as_str(), *name);
    }
}

/// Epic #2481 T3: a refusal says, for the call's record, which schema
/// field was wrong and how; the request's own text never names it.
#[test]
fn a_refusal_records_its_schema_field_and_fault() {
    use crate::interface::tools::project_board::{MAX_REQUEST_BYTES, read};
    let fault = |error: RequestError| {
        let RequestFault {
            field,
            fault,
            unexpected_count,
        } = error.fault;
        (field.as_str(), fault.as_str(), unexpected_count)
    };
    let cases = [
        (json!({"op": "show"}), ("task", "missing", None)),
        (
            json!({"op": "show", "task": 1}),
            ("task", "wrong_type", None),
        ),
        (
            json!({"op": "show", "task": "Not A Slug"}),
            ("task", "invalid_value", None),
        ),
        (
            json!({"op": "move", "task": "a", "to": "Away"}),
            ("to", "invalid_value", None),
        ),
        (
            json!({"op": "link_pr", "task": "a", "pr": "7"}),
            ("pr", "wrong_type", None),
        ),
        (
            json!({"op": "link_pr", "task": "a", "pr": 0}),
            ("pr", "invalid_value", None),
        ),
        (
            json!({"op": "create", "task": "a", "title": "A", "depends_on": ["a", 2]}),
            ("depends_on", "wrong_type", None),
        ),
        (
            json!({"op": "list", "status": "ready", "parent": "a"}),
            ("parent", "invalid_value", None),
        ),
        (
            json!({"op": "update_draft", "task": "a"}),
            ("op", "missing", None),
        ),
        (json!({"op": "Drop Tables"}), ("op", "invalid_value", None)),
        (json!({"op": 3}), ("op", "wrong_type", None)),
        (json!({"task": "a"}), ("op", "missing", None)),
        (json!(["op"]), ("request", "wrong_type", None)),
        (
            json!({"op": "clear_set_aside", "path": "x", "object": "0".repeat(40), "repeated": {"Secret Path": {"drop": ["Secret"]}}}),
            ("repeated", "invalid_value", None),
        ),
        (
            json!({"op": "clear_set_aside", "path": "x".repeat(10_000), "object": "0".repeat(40)}),
            ("path", "too_large", None),
        ),
        // A stray key is `other`, counted, even when another op takes it.
        (
            json!({"op": "show", "task": "a", "title": "A"}),
            ("other", "unexpected", Some(1)),
        ),
        (
            json!({"op": "list", "Secret Key": 1, "title": 2}),
            ("other", "unexpected", Some(2)),
        ),
    ];
    for (value, want) in cases {
        assert_eq!(fault(refused(value.clone())), want, "{value}");
    }
    let padded = format!(r#"{{"op": "list"}}{}"#, " ".repeat(MAX_REQUEST_BYTES));
    let texts = [
        (r#"{"op": "show" "#, ("request", "not_json", None)),
        (
            r#"{"op": "show", "op": "list"}"#,
            ("request", "duplicate_key", None),
        ),
        (padded.as_str(), ("request", "too_large", None)),
    ];
    for (text, want) in texts {
        assert_eq!(fault(read(text).unwrap_err()), want, "{}", &text[..20]);
    }
}

/// Only a field of `OPS`, or `op` or `request`, is recorded by name, as
/// the table spells it; anything else is `other`.
#[test]
fn a_schema_field_is_one_the_table_spells() {
    for (field, want) in [
        ("depends_on/3", "depends_on"),
        ("repeated/Secret Path/keep", "repeated"),
        ("expected_updated", "expected_updated"),
        ("request", "request"),
        ("Title", "other"),
        ("flags", "other"),
        ("", "other"),
        ("ok", "other"),
    ] {
        assert_eq!(schema_field(field).as_str(), want, "{field:?}");
    }
}

/// S4c: end_run reads its run, its outcome (an allowlist) and its
/// optional pin; a run out of range or an outcome off the list is refused.
#[test]
fn end_run_reads_its_run_outcome_and_pin() {
    use crate::application::project_board::dto::{RunEnd, RunEndOutcome};
    let at = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    assert_eq!(
        parse(
            &json!({"op": "end_run", "task": "a", "run": 2, "outcome": "stopped",
                      "expected_updated": "2026-10-10T09:00:00Z"})
        ),
        Ok(BoardRequest::EndRun(RunEnd {
            task: slug("a"),
            run: 2,
            outcome: RunEndOutcome::Stopped,
            expected_updated: Some(at),
        }))
    );
    for (value, field, problem) in [
        (
            json!({"op": "end_run", "task": "a", "run": 1, "outcome": "completed"}),
            "outcome",
            "must be one of succeeded, stopped, failed, lost",
        ),
        (
            json!({"op": "end_run", "task": "a", "run": 0, "outcome": "lost"}),
            "run",
            "must be a run number from 1 to 999",
        ),
        (
            json!({"op": "end_run", "task": "a", "run": 1000, "outcome": "lost"}),
            "run",
            "must be a run number from 1 to 999",
        ),
    ] {
        let error = refused(value.clone());
        assert_eq!(
            (error.field.as_str(), error.problem.as_str()),
            (field, problem),
            "{value}"
        );
    }
}
