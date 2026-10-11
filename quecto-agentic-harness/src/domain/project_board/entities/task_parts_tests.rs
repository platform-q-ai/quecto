use super::*;
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::team::*;
use crate::domain::project_board::value_objects::vocabulary::Effort;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};

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

fn item(id: &str, depends_on: &[&str]) -> PlanItem {
    PlanItem {
        id: slug(id),
        title: format!("Item {id}"),
        criteria: vec!["passes".into()],
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
    }
}

/// A plan with every part: the items and team a task's swarm runs.
fn plan_fields() -> PlanFields {
    PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Draft,
        approach: String::new(),
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
        proposed_by: ada(),
        accepted: None,
        created: at("08:00:00"),
        updated: at("10:00:00"),
    }
}

/// A task with its review findings.
fn fields() -> TaskFields {
    TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::InProgress,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: Some(Claim {
            holder: ada(),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        reviews: vec![Review {
            id: slug("r1-h1"),
            round: 1,
            reviewer: ada(),
            at: at("10:00:00"),
            severity: Severity::High,
            location: Some("src/store.rs:42:7".into()),
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
type PlanEdit = fn(&mut PlanFields);

fn team(plan: &mut PlanFields) -> &mut Team {
    plan.team.as_mut().unwrap()
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
fn the_full_task_and_plan_build() {
    assert!(Task::new(fields()).is_ok());
    assert!(Plan::new(plan_fields()).is_ok());
}

#[test]
fn plan_parts_that_break_a_rule_build_no_plan() {
    let cases: Vec<(&str, PlanEdit)> = vec![
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
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "a/.b".into()
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "a//b".into()
        }),
        ("team/roles/0/model", |t| {
            team(t).roles[0].model = "a/".into()
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
        ("team/budget/tokens", |t| {
            team(t).budget = Some(Budget {
                tokens: MAX_BUDGET_TOKENS + 1,
            })
        }),
        ("team/deadline", |t| team(t).deadline = Some(at("08:00:00"))),
        ("team/deadline", |t| team(t).deadline = Some(at("07:00:00"))),
        ("team/image", |t| team(t).image = Some("Foo/bar".into())),
        ("team/image", |t| {
            team(t).image = Some("registry/Team".into())
        }),
        ("team/image", |t| team(t).image = Some("Quecto".into())),
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
    ];
    for (field, edit) in cases {
        let mut plan = plan_fields();
        edit(&mut plan);
        let error = Plan::new(plan).expect_err(field);
        assert_eq!(error.field, field, "{error}");
    }
}

#[test]
fn reviews_that_break_a_rule_build_no_task() {
    let cases: Vec<(&str, Edit)> = vec![
        ("reviews", |t| t.reviews.push(t.reviews[0].clone())),
        ("reviews/0/round", |t| t.reviews[0].round = 0),
        ("reviews/0/at", |t| t.reviews[0].at = at("07:59:59")),
        ("reviews/0/at", |t| t.reviews[0].at = at("10:00:01")),
        ("reviews/0/reviewer/email", |t| {
            t.reviews[0].reviewer.email = "ada".into()
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("src/store.rs".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("src/store.rs:0".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("src/store.rs:042".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("src/store.rs:4:0".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("src/a b.rs:4".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("../etc/passwd:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("-rf.rs:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("./a.rs:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("a/./b.rs:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("a//b.rs:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("a/:1".into())
        }),
        ("reviews/0/location", |t| {
            t.reviews[0].location = Some("/etc/passwd:1".into())
        }),
        ("reviews/0/finding", |t| {
            t.reviews[0].finding = String::new()
        }),
        ("reviews/0/finding", |t| {
            t.reviews[0].finding = "\n\t".into()
        }),
        ("reviews/0/finding", |t| {
            t.reviews[0].finding = "x".repeat(MAX_FINDING_CHARS + 1)
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
    let plans: Vec<PlanEdit> = vec![
        |t| t.items = many(&t.items[0], MAX_ITEMS, |i, n| i.id = slug(&format!("i{n}"))),
        |t| t.items[0].criteria = (0..MAX_PER_ITEM).map(|n| n.to_string()).collect(),
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
        |t| team(t).image = Some("ghcr.io/team/app:1..2".into()),
        |t| team(t).roles[0].model = "fireworks/models/glm-4.6:thinking".into(),
        |t| team(t).roles[0].model = "gpt..5".into(),
        |t| team(t).deadline = Some(at("08:00:01")),
        |t| {
            team(t).budget = Some(Budget {
                tokens: MAX_BUDGET_TOKENS,
            })
        },
        |t| team(t).image = Some("quecto".into()),
        |t| team(t).image = Some("library/ubuntu:24.04".into()),
        |t| team(t).image = Some("localhost/x".into()),
        |t| team(t).image = Some("registry:5000/team/app_1__b".into()),
        |t| team(t).image = Some("Registry.Example.com/team/app".into()),
        |t| team(t).image = Some("[::1]:5000/x".into()),
        |t| (t.team, t.items) = (None, vec![]),
    ];
    for (index, edit) in plans.into_iter().enumerate() {
        let mut plan = plan_fields();
        edit(&mut plan);
        assert!(
            Plan::new(plan).map_err(|error| error.to_string()).is_ok(),
            "plan case {index}"
        );
    }
    let tasks: Vec<Edit> = vec![
        |t| {
            t.reviews = many(&t.reviews[0], MAX_REVIEWS, |r, n| {
                r.id = slug(&format!("f{n}"))
            })
        },
        |t| t.reviews[0].location = Some("src/store.rs:1".into()),
        |t| t.reviews[0].location = Some(".github/ci.yml:3".into()),
        |t| t.reviews[0].location = Some("src/a..b.rs:1".into()),
        |t| t.reviews[0].location = Some("a/-b.rs:1".into()),
        |t| t.reviews[0].location = None,
        |t| t.reviews[0].finding = "é".repeat(MAX_FINDING_CHARS),
        |t| t.reviews[0].finding = "**Lease**\n\n\tnot checked".into(),
        |t| t.reviews[0].verdict = Verdict::WontFix,
        |t| t.reviews[0].at = at("08:00:00"),
        |t| t.status = TaskStatus::Review,
        |t| t.status = TaskStatus::Blocked,
        |t| (t.status, t.claim) = (TaskStatus::Review, None),
    ];
    for (index, edit) in tasks.into_iter().enumerate() {
        let mut task = fields();
        edit(&mut task);
        assert!(
            Task::new(task).map_err(|error| error.to_string()).is_ok(),
            "task case {index}"
        );
    }
}
