use super::*;
use serde_json::{Value, json};

fn full_json() -> Value {
    json!({
        "id": "board-store", "title": "Board store", "kind": "task",
        "description": "Store **tasks** on a branch.\n\n- one\n- two", "status": "in_progress",
        "parent": "boards", "children": ["board-index"], "depends_on": ["task-schema"],
        "claim": {"holder": {"name": "Ada", "email": "ada@example.com"},
                  "since": "2026-10-10T09:00:00Z", "expires": "2026-10-10T11:00:00Z"},
        "pr": 2490, "created": "2026-10-10T08:00:00Z", "updated": "2026-10-10T09:05:00Z"
    })
}

fn full_task() -> Task {
    let task: Task = serde_json::from_value(full_json()).expect("full task parses");
    task.validate().expect("full task is valid");
    task
}

#[test]
fn a_task_serialises_every_field_in_schema_order_and_round_trips() {
    let task = full_task();
    let text = serde_json::to_string_pretty(&task).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let schema =
        "id title kind description status parent children depends_on claim pr created updated";
    assert_eq!(keys, schema.split(' ').collect::<Vec<_>>());
    assert_eq!(
        value,
        full_json(),
        "nothing is renamed, defaulted or dropped"
    );
    assert_eq!(serde_json::from_str::<Task>(&text).unwrap(), task);
    assert_eq!(
        serde_json::to_string_pretty(&task).unwrap(),
        text,
        "formatting is stable"
    );
}

#[test]
fn unknown_fields_and_values_outside_the_vocabularies_are_refused() {
    let edits = [
        ("extra", json!(1)),
        ("kind", json!("story")),
        ("status", json!("wip")),
    ];
    for (key, replacement) in edits {
        let mut value = full_json();
        value
            .as_object_mut()
            .unwrap()
            .insert(key.into(), replacement);
        assert!(
            serde_json::from_value::<Task>(value).is_err(),
            "{key} must be refused"
        );
    }
    for spelling in [
        "draft ready claimed in_progress review blocked done archived",
        "epic task chore",
    ] {
        for word in spelling.split(' ') {
            let field = if spelling.starts_with("epic") {
                "kind"
            } else {
                "status"
            };
            let mut value = full_json();
            value[field] = json!(word);
            let task: Task = serde_json::from_value(value).expect(word);
            assert_eq!(serde_json::to_value(&task).unwrap()[field], json!(word));
        }
    }
}

/// One change that must make a valid task invalid.
type Edit = fn(&mut Task);

#[test]
fn validation_refuses_out_of_bounds_and_inconsistent_tasks() {
    let cases: Vec<(&str, Edit)> = vec![
        ("title", |t| t.title = String::new()),
        ("title", |t| t.title = "two\nlines".into()),
        ("title", |t| t.title = "x".repeat(MAX_TITLE_CHARS + 1)),
        ("description", |t| {
            t.description = "x".repeat(MAX_DESCRIPTION_BYTES + 1)
        }),
        ("description", |t| t.description = "bell\u{7}".into()),
        ("children", |t| {
            t.children = (0..=MAX_LIST)
                .map(|i| Slug::parse(&format!("c{i}")).unwrap())
                .collect()
        }),
        ("children", |t| t.children.push(t.children[0].clone())),
        ("children", |t| t.children.push(t.id.clone())),
        ("parent", |t| t.parent = Some(t.id.clone())),
        ("depends_on", |t| t.depends_on.push(t.id.clone())),
        ("claim", |t| t.claim = None),
        ("claim", |t| t.status = TaskStatus::Ready),
        ("claim", |t| t.status = TaskStatus::Done),
        ("claim", |t| {
            let c = t.claim.as_mut().unwrap();
            c.expires = c.since.clone()
        }),
        ("claim/holder", |t| {
            t.claim.as_mut().unwrap().holder.email = "ada".into()
        }),
        ("claim/holder", |t| {
            t.claim.as_mut().unwrap().holder.email = "a@b@c".into()
        }),
        ("claim/holder", |t| {
            t.claim.as_mut().unwrap().holder.name = String::new()
        }),
        ("pr", |t| t.pr = Some(0)),
        ("updated", |t| {
            t.updated = Timestamp::parse("2026-10-10T07:59:59Z").unwrap()
        }),
    ];
    for (field, edit) in cases {
        let mut task = full_task();
        edit(&mut task);
        let error = task.validate().expect_err(field);
        assert_eq!(error.field, field, "{error}");
    }
}

#[test]
fn a_claim_is_carried_only_while_the_task_is_held() {
    use TaskStatus::*;
    let allowed = [
        (Draft, false),
        (Ready, false),
        (Claimed, true),
        (InProgress, true),
        (Review, true),
        (Review, false),
        (Blocked, true),
        (Blocked, false),
        (Done, false),
        (Archived, false),
    ];
    for (status, claimed) in allowed {
        let mut task = full_task();
        task.status = status;
        if !claimed {
            task.claim = None;
        }
        assert_eq!(task.validate(), Ok(()), "{status:?} claimed={claimed}");
    }
}
