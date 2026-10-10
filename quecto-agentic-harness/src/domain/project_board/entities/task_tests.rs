use super::*;
use crate::domain::project_board::entities::claim::{CLAIM_TTL_SECONDS, Identity};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn fields() -> TaskFields {
    TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: "Store **tasks** on a branch.\n\n- one".into(),
        status: TaskStatus::InProgress,
        parent: Some(slug("boards")),
        depends_on: vec![slug("task-schema")],
        claim: Some(Claim {
            holder: Identity {
                name: "Ada Lovelace".into(),
                email: "Ada@Example.com".into(),
            },
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        prs: vec![2482, 2483],
        created: at("08:00:00"),
        updated: at("09:05:00"),
    }
}

#[test]
fn valid_fields_build_a_task_and_read_back_unchanged() {
    let task = Task::new(fields()).expect("valid");
    assert_eq!(task.fields(), &fields());
    assert_eq!(
        Task::try_from(fields()).map(Task::into_fields),
        Ok(fields())
    );
}

/// One change that must make valid fields invalid.
type Edit = fn(&mut TaskFields);

fn claim(task: &mut TaskFields) -> &mut Claim {
    task.claim.as_mut().unwrap()
}

#[test]
fn fields_that_break_a_rule_build_no_task() {
    let cases: Vec<(&str, Edit)> = vec![
        ("title", |t| t.title = String::new()),
        ("title", |t| t.title = "two\nlines".into()),
        ("title", |t| t.title = "x".repeat(MAX_TITLE_CHARS + 1)),
        ("description", |t| {
            t.description = "é".repeat(MAX_DESCRIPTION_CHARS + 1)
        }),
        ("description", |t| t.description = "crlf\r\n".into()),
        ("parent", |t| t.parent = Some(t.id.clone())),
        ("depends_on", |t| {
            t.depends_on = (0..=MAX_DEPENDENCIES)
                .map(|i| slug(&format!("d{i}")))
                .collect()
        }),
        ("depends_on", |t| t.depends_on.push(t.depends_on[0].clone())),
        ("depends_on/1", |t| t.depends_on.push(t.id.clone())),
        ("claim", |t| t.claim = None),
        ("claim", |t| t.status = TaskStatus::Ready),
        ("claim", |t| t.status = TaskStatus::Done),
        ("claim", |t| claim(t).expires = at("09:00:00")),
        ("claim", |t| claim(t).expires = at("11:00:01")),
        ("claim/since", |t| {
            (claim(t).since, claim(t).expires) = (at("07:59:59"), at("09:00:00"))
        }),
        ("claim/holder/name", |t| {
            claim(t).holder.name = String::new()
        }),
        ("claim/holder/name", |t| {
            claim(t).holder.name = "Ada <ada@x>".into()
        }),
        ("claim/holder/name", |t| {
            claim(t).holder.name = "Ada.".into()
        }),
        ("claim/holder/name", |t| {
            claim(t).holder.name = " Ada".into()
        }),
        ("claim/holder/name", |t| {
            claim(t).holder.name = "x".repeat(101)
        }),
        ("claim/holder/email", |t| {
            claim(t).holder.email = "ada".into()
        }),
        ("claim/holder/email", |t| {
            claim(t).holder.email = "a@b@c".into()
        }),
        ("claim/holder/email", |t| {
            claim(t).holder.email = "<ada@example.com>".into()
        }),
        ("claim/holder/email", |t| {
            claim(t).holder.email = "ada@example.com\n".into()
        }),
        ("claim/holder/email", |t| {
            claim(t).holder.email = format!("{}@x.io", "a".repeat(250))
        }),
        ("prs", |t| t.prs.push(t.prs[0])),
        ("prs", |t| t.prs = (1..=MAX_PRS as u64 + 1).collect()),
        ("prs/1", |t| t.prs[1] = 0),
        ("updated", |t| t.updated = at("07:59:59")),
    ];
    for (field, edit) in cases {
        let mut task = fields();
        edit(&mut task);
        let error = Task::new(task).expect_err(field);
        assert_eq!(error.field, field, "{error}");
    }
}

#[test]
fn every_bound_admits_exactly_its_limit() {
    let cases: Vec<Edit> = vec![
        |t| t.title = "é".repeat(MAX_TITLE_CHARS),
        |t| t.description = "日".repeat(MAX_DESCRIPTION_CHARS),
        |t| {
            t.depends_on = (0..MAX_DEPENDENCIES)
                .map(|i| slug(&format!("d{i}")))
                .collect()
        },
        |t| t.prs = (1..=MAX_PRS as u64).collect(),
        |t| claim(t).expires = claim(t).since.plus_seconds(CLAIM_TTL_SECONDS).unwrap(),
        |t| (claim(t).since, claim(t).expires) = (at("08:00:00"), at("10:00:00")),
        |t| claim(t).holder.name = format!("A{}", "x".repeat(99)),
        |t| claim(t).holder.email = format!("{}@x.io", "a".repeat(249)),
        |t| t.description = String::new(),
        |t| (t.parent, t.depends_on, t.prs) = (None, vec![], vec![]),
    ];
    for (index, edit) in cases.into_iter().enumerate() {
        let mut task = fields();
        edit(&mut task);
        assert!(Task::new(task).is_ok(), "case {index}");
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
        let mut task = fields();
        task.status = status;
        if !claimed {
            task.claim = None;
        }
        assert!(Task::new(task).is_ok(), "{status:?} claimed={claimed}");
    }
}

#[test]
fn a_committer_is_known_by_email_whatever_its_case() {
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let shouting = Identity {
        name: "ADA".into(),
        email: "ADA@EXAMPLE.COM".into(),
    };
    let bob = Identity {
        name: "Ada".into(),
        email: "bob@example.com".into(),
    };
    assert!(ada.is(&shouting));
    assert!(!ada.is(&bob));
}
