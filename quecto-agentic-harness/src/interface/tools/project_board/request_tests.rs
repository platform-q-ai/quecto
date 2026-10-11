use super::*;
use serde_json::json;

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
            json!({"op": "clear_set_aside", "path": "tasks/x.json"}),
            BoardRequest::Clear {
                path: "tasks/x.json".into(),
                keep: None,
            },
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "keep": "a".repeat(40)}),
            BoardRequest::Clear {
                path: "tasks/x.json".into(),
                keep: Some("a".repeat(40)),
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
fn create_takes_a_plan_and_defaults_to_a_draft_task() {
    let created = parse(&json!({
        "op": "create",
        "task": "a",
        "title": "Board CLI",
        "description": "Why.\r\nHow.\r\n",
        "depends_on": ["b"],
        "plan": [{"title": "Spike", "criteria": ["Proved"]}, {"title": "Ship"}],
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
    assert_eq!(
        new.plan,
        vec![
            PlanStep {
                title: "Spike".into(),
                criteria: vec!["Proved".into()]
            },
            PlanStep {
                title: "Ship".into(),
                criteria: vec![]
            },
        ]
    );
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
            plan: None,
            parent: Some(None),
            depends_on: None,
            expected_updated: Some(Timestamp::parse("2026-10-10T08:00:00Z").unwrap()),
        }
    );
    assert_eq!(
        refused(json!({"op": "update_draft", "task": "a"})).field,
        "update_draft"
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
        (
            json!({"op": "create", "task": "a", "title": "T", "plan": [{"title": "S", "owner": "x"}]}),
            "plan/0/owner",
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
            json!({"op": "clear_set_aside", "path": "tasks/x.json", "keep": "HEAD"}),
            "keep",
        ),
        (
            json!({"op": "clear_set_aside", "path": "README.md"}),
            "path",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/../x"}),
            "path",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/.hidden"}),
            "path",
        ),
        (
            json!({"op": "clear_set_aside", "path": "tasks/a/b.json"}),
            "path",
        ),
    ] {
        assert_eq!(refused(value.clone()).field, field, "{value}");
    }
}
