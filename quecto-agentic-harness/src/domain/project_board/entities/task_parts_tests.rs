use super::*;
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

fn item(id: &str, depends_on: &[&str]) -> Item {
    Item {
        id: slug(id),
        title: format!("Item {id}"),
        criteria: vec!["passes".into()],
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
        state: ItemState::Todo,
        owner: Some("worker-1".into()),
        evidence: vec!["log".into()],
    }
}

fn fields() -> TaskFields {
    TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::InProgress,
        parent: None,
        depends_on: vec![],
        plan: vec![PlanStep {
            title: "Spike".into(),
            criteria: vec!["CAS proved".into()],
        }],
        items: vec![
            item("plumbing", &[]),
            item("cas", &["plumbing"]),
            item("index", &["plumbing", "cas"]),
        ],
        team: Some(Team {
            roles: vec![Role {
                name: slug("worker"),
                model: "openai/gpt-5.5".into(),
                effort: Effort::High,
                limits: RoleLimits {
                    members: 2,
                    turns: Some(200),
                },
            }],
            budget: Some(Budget { tokens: 5_000_000 }),
            deadline: Some(at("23:00:00")),
            image: Some("ghcr.io/platform-q-ai/quecto:0.107".into()),
        }),
        claim: Some(Claim {
            holder: ada(),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        runs: vec![
            Run {
                id: slug("r1"),
                started: at("09:01:00"),
                ended: Some(at("09:30:00")),
                outcome: Some(RunOutcome::Lost),
            },
            Run {
                id: slug("r2"),
                started: at("09:31:00"),
                ended: None,
                outcome: None,
            },
        ],
        reviews: vec![Review {
            id: slug("r1-h1"),
            round: 1,
            reviewer: ada(),
            at: at("10:00:00"),
            severity: Severity::High,
            location: "src/store.rs:42:7".into(),
            finding: "Lease not checked".into(),
            verdict: Verdict::Fixed,
            proving_test: Some("stale_lease_is_refused".into()),
        }],
        prs: vec![2482],
        created: at("08:00:00"),
        updated: at("10:00:00"),
    }
}

/// One change that must make valid fields invalid.
type Edit = fn(&mut TaskFields);

fn team(task: &mut TaskFields) -> &mut Team {
    task.team.as_mut().unwrap()
}

fn many<T: Clone>(entry: &T, count: usize, rename: fn(&mut T, usize)) -> Vec<T> {
    (0..count)
        .map(|index| {
            let mut next = entry.clone();
            rename(&mut next, index);
            next
        })
        .collect()
}

#[test]
fn the_full_task_builds() {
    assert!(Task::new(fields()).is_ok());
}

#[test]
fn parts_that_break_a_rule_build_no_task() {
    let cases: Vec<(&str, Edit)> = vec![
        ("plan", |t| t.plan = vec![t.plan[0].clone(); MAX_STEPS + 1]),
        ("plan/0/title", |t| t.plan[0].title = "\t".into()),
        ("plan/0/criteria", |t| {
            t.plan[0].criteria = vec!["c".into(); 2]
        }),
        ("items", |t| t.items[1].id = t.items[0].id.clone()),
        ("items", |t| {
            t.items = many(&t.items[0], MAX_ITEMS + 1, |i, n| {
                i.id = slug(&format!("i{n}"))
            })
        }),
        ("items/0/title", |t| {
            t.items[0].title = "x".repeat(MAX_LINE_CHARS + 1)
        }),
        ("items/0/criteria", |t| {
            t.items[0].criteria = (0..=MAX_PER_ITEM).map(|n| n.to_string()).collect()
        }),
        ("items/0/criteria/0", |t| {
            t.items[0].criteria = vec![" ".into()]
        }),
        ("items/0/owner", |t| t.items[0].owner = Some("a\nb".into())),
        ("items/0/evidence", |t| {
            t.items[0].evidence = vec!["same".into(), "same".into()]
        }),
        ("items/1/depends_on", |t| {
            t.items[1].depends_on = vec![slug("missing")]
        }),
        ("items/0/depends_on", |t| {
            t.items[0].depends_on = vec![slug("plumbing")]
        }),
        ("items/0/depends_on", |t| {
            t.items[0].depends_on = vec![slug("index")]
        }),
        ("team/roles", |t| team(t).roles.clear()),
        ("team/roles", |t| {
            let role = team(t).roles[0].clone();
            team(t).roles.push(role)
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "-gpt".into()
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "a/-gpt".into()
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "a/../b".into()
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "gpt 5".into()
        }),
        ("team/roles/0/limits/members", |t| {
            team(t).roles[0].limits.members = 0
        }),
        ("team/roles/0/limits/members", |t| {
            team(t).roles[0].limits.members = MAX_MEMBERS + 1
        }),
        ("team/roles/0/limits/turns", |t| {
            team(t).roles[0].limits.turns = Some(0)
        }),
        ("team/budget/tokens", |t| {
            team(t).budget = Some(Budget { tokens: 0 })
        }),
        ("team/image", |t| {
            team(t).image = Some("img; rm -rf /".into())
        }),
        ("team/image", |t| team(t).image = Some("-ghcr.io/x".into())),
        ("team/image", |t| {
            team(t).image = Some("ghcr.io/a/../b".into())
        }),
        ("team/image", |t| {
            team(t).image = Some("Ghcr.io/Upper".into())
        }),
        ("runs", |t| t.runs.push(t.runs[0].clone())),
        ("runs", |t| {
            t.runs[0] = Run {
                ended: None,
                outcome: None,
                ..t.runs[0].clone()
            }
        }),
        ("runs", |t| t.status = TaskStatus::Review),
        ("runs/0", |t| t.runs[0].outcome = None),
        ("runs/1", |t| t.runs[1].outcome = Some(RunOutcome::Failed)),
        ("runs/0", |t| {
            t.runs[0].ended = Some(t.runs[0].started.clone())
        }),
        ("reviews", |t| t.reviews.push(t.reviews[0].clone())),
        ("reviews/0/round", |t| t.reviews[0].round = 0),
        ("reviews/0/reviewer/email", |t| {
            t.reviews[0].reviewer.email = "ada".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "src/store.rs".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "src/store.rs:0".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "src/store.rs:042".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "src/store.rs:4:0".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "src/a b.rs:4".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = "../etc/passwd:1".into()
        }),
        ("reviews/0/finding", |t| {
            t.reviews[0].finding = String::new()
        }),
        ("reviews/0/proving_test", |t| {
            t.reviews[0].proving_test = Some(" ".into())
        }),
    ];
    for (field, edit) in cases {
        let mut task = fields();
        edit(&mut task);
        let error = Task::new(task).expect_err(field);
        assert_eq!(error.field, field, "{error}");
    }
}

#[test]
fn every_part_bound_admits_exactly_its_limit() {
    let cases: Vec<Edit> = vec![
        |t| t.plan = many(&t.plan[0], MAX_STEPS, |s, n| s.title = format!("step {n}")),
        |t| t.items = many(&t.items[0], MAX_ITEMS, |i, n| i.id = slug(&format!("i{n}"))),
        |t| t.items[0].criteria = (0..MAX_PER_ITEM).map(|n| n.to_string()).collect(),
        |t| t.items[0].evidence = (0..MAX_PER_ITEM).map(|n| n.to_string()).collect(),
        |t| t.items[0].owner = Some("x".repeat(MAX_OWNER_CHARS)),
        |t| t.items[2].title = "é".repeat(MAX_LINE_CHARS),
        |t| {
            team(t).roles = many(&team(t).roles[0], MAX_ROLES, |r, n| {
                r.name = slug(&format!("r{n}"))
            })
        },
        |t| {
            team(t).roles[0].limits = RoleLimits {
                members: MAX_MEMBERS,
                turns: None,
            }
        },
        |t| team(t).roles[0].model = "m".repeat(MAX_MODEL_BYTES),
        |t| team(t).image = Some(format!("a/{}", "b".repeat(MAX_IMAGE_BYTES - 2))),
        |t| team(t).image = Some(format!("localhost:5000/x/y:v1.2@sha256:{}", "a".repeat(64))),
        |t| (t.team, t.plan, t.items) = (None, vec![], vec![]),
        |t| t.runs = many(&t.runs[0], MAX_RUNS, |r, n| r.id = slug(&format!("r{n}"))),
        |t| {
            t.reviews = many(&t.reviews[0], MAX_REVIEWS, |r, n| {
                r.id = slug(&format!("f{n}"))
            })
        },
        |t| t.reviews[0].location = "src/store.rs:1".into(),
        |t| t.reviews[0].verdict = Verdict::WontFix,
    ];
    for (index, edit) in cases.into_iter().enumerate() {
        let mut task = fields();
        edit(&mut task);
        assert!(
            Task::new(task).map_err(|error| error.to_string()).is_ok(),
            "case {index}"
        );
    }
}
