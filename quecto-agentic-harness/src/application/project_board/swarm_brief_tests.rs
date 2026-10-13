use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{PlanFields, PlanItem};
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::telemetry::StaticKind;
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

/// The fixture task, in progress under Ada's claim, its plan v1 accepted.
fn task() -> Task {
    Task::new(TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::InProgress,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: Some(crate::domain::project_board::entities::claim::Claim {
            holder: ada(),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at("08:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap()
}

fn item(id: &str, criteria: &[&str], depends_on: &[&str]) -> PlanItem {
    PlanItem {
        id: slug(id),
        title: format!("Build the {id}"),
        criteria: criteria.iter().map(|line| (*line).to_owned()).collect(),
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
    }
}

fn role(name: &str, model: &str, members: u16) -> Role {
    Role {
        name: slug(name),
        model: model.into(),
        effort: Effort::High,
        limits: RoleLimits {
            members,
            turns: None,
        },
    }
}

/// Accepted version 1 of the task's plan: `api` listed before the
/// `schema` it depends on, and `team`.
fn plan_with(items: Vec<PlanItem>, team: Option<Team>, approach: &str) -> Plan {
    Plan::new(PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Accepted,
        approach: approach.into(),
        items,
        team,
        proposed_by: ada(),
        edited_by: None,
        accepted: Some((ada(), at("08:30:00"))),
        created: at("08:30:00"),
        updated: at("08:30:00"),
    })
    .unwrap()
}

fn team(roles: Vec<Role>) -> Team {
    Team {
        roles,
        budget: Some(Budget { tokens: 2_000_000 }),
        deadline: Some(at("16:00:00")),
        image: None,
    }
}

fn items() -> Vec<PlanItem> {
    vec![
        item(
            "api",
            &["serves GET /tasks", "answers 404 for a missing task"],
            &["schema"],
        ),
        item("schema", &[], &[]),
        item("docs", &["documents the API"], &["api", "schema"]),
    ]
}

/// Sixteen distinct criteria of 500 characters, each mostly `fill`.
fn long_lines(fill: &str) -> Vec<String> {
    (0..16)
        .map(|at| format!("{at:02} {}", fill.repeat(497)))
        .collect()
}

fn full_team() -> Team {
    team(vec![
        role("coordinator", "claude-opus", 1),
        role("builder", "gpt-5.5", 2),
        role("reviewer", "glm-5", 1),
    ])
}

/// The whole mapping of a plan, as §3.2 of the shortcut plan lays it out.
#[test]
fn a_plan_maps_onto_the_swarms_inputs() {
    let plan = plan_with(items(), Some(full_team()), "Spike, then build.");
    let brief = brief_of(&task(), &plan, &at("10:00:00")).unwrap();
    let want = SwarmBrief {
        task: slug("board-store"),
        plan_version: 1,
        goal: "Board task board-store, plan v1: Board store".into(),
        constraints: vec![
            "Spike, then build.".into(),
            "The plan in full, approach included: the board's show op for task board-store \
             (plan v1)."
                .into(),
        ],
        approach_cut: false,
        criteria: RUN_CRITERIA.to_vec(),
        member_limit: Some(4),
        deadline_in_seconds: 6 * 3600,
        usage_budget: Some(2_000_000),
        items: vec![
            BriefItem {
                id: slug("schema"),
                title: "schema: Build the schema".into(),
                acceptance: vec!["Build the schema".into()],
                dependencies: vec![],
            },
            BriefItem {
                id: slug("api"),
                title: "api: Build the api".into(),
                acceptance: vec![
                    "serves GET /tasks".into(),
                    "answers 404 for a missing task".into(),
                ],
                dependencies: vec![slug("schema")],
            },
            BriefItem {
                id: slug("docs"),
                title: "docs: Build the docs".into(),
                acceptance: vec!["documents the API".into()],
                dependencies: vec![slug("api"), slug("schema")],
            },
        ],
        coordinator: Some(RoleChoice {
            model: "claude-opus".into(),
            effort: Effort::High,
        }),
        not_applied: vec![
            NotApplied {
                role: slug("builder"),
                model: "gpt-5.5".into(),
                effort: Effort::High,
            },
            NotApplied {
                role: slug("reviewer"),
                model: "glm-5".into(),
                effort: Effort::High,
            },
        ],
    };
    assert_eq!(brief, want);
    assert!(brief.bytes() <= MAX_BRIEF_BYTES);
}

/// No team: the coordinator picks its pool, its model is the launcher's,
/// and the run has a day.
#[test]
fn a_plan_without_a_team_leaves_the_pool_to_the_coordinator() {
    let brief = brief_of(&task(), &plan_with(items(), None, ""), &at("10:00:00")).unwrap();
    assert_eq!(brief.member_limit, None);
    assert_eq!(brief.deadline_in_seconds, 24 * 3600);
    assert_eq!(brief.usage_budget, None);
    assert_eq!(brief.coordinator, None);
    assert!(brief.not_applied.is_empty());
    // A blank approach is no constraint; the pointer to the plan stays.
    assert_eq!(brief.constraints.len(), 1);
}

/// The swarm holds at most 25 members, its coordinator one of them.
#[test]
fn the_member_limit_is_the_coordinator_and_every_worker() {
    let limit = |workers: &[u16]| {
        let roles = workers
            .iter()
            .enumerate()
            .map(|(at, members)| role(&format!("pool-{at}"), "gpt-5.5", *members))
            .collect();
        brief_of(
            &task(),
            &plan_with(items(), Some(team(roles)), ""),
            &at("10:00:00"),
        )
        .map(|brief| brief.member_limit)
    };
    assert_eq!(limit(&[1]), Ok(Some(2)));
    assert_eq!(limit(&[12, 12]), Ok(Some(25)));
    assert_eq!(
        limit(&[12, 13]),
        Err(BriefRefusal::TeamTooLarge { members: 26 })
    );
    assert_eq!(
        limit(&[32, 32]),
        Err(BriefRefusal::TeamTooLarge { members: 65 })
    );
}

/// A deadline is the plan's less now, at most seven days; one passed is
/// refused.
#[test]
fn the_deadline_is_clamped_to_a_week_and_never_passed() {
    let deadline = |deadline: &str| {
        let mut with = full_team();
        with.deadline = Some(Timestamp::parse(deadline).unwrap());
        brief_of(
            &task(),
            &plan_with(items(), Some(with), ""),
            &at("10:00:00"),
        )
        .map(|brief| brief.deadline_in_seconds)
    };
    assert_eq!(deadline("2026-10-10T10:00:01Z"), Ok(1));
    assert_eq!(deadline("2026-10-17T10:00:00Z"), Ok(7 * 24 * 3600));
    assert_eq!(deadline("2027-01-01T00:00:00Z"), Ok(7 * 24 * 3600));
    for passed in ["2026-10-10T10:00:00Z", "2026-10-09T10:00:00Z"] {
        assert_eq!(
            deadline(passed),
            Err(BriefRefusal::DeadlinePassed),
            "{passed}"
        );
    }
}

/// What spawn cannot carry yet is refused, never dropped.
#[test]
fn a_team_spawn_cannot_carry_is_refused() {
    let refused = |team: Team| {
        brief_of(
            &task(),
            &plan_with(items(), Some(team), ""),
            &at("10:00:00"),
        )
    };
    let mut imaged = full_team();
    imaged.image = Some("ghcr.io/platform-q-ai/quecto:1".into());
    assert_eq!(
        refused(imaged),
        Err(BriefRefusal::TeamUnsupported {
            field: TeamField::Image
        })
    );
    let mut limited = full_team();
    limited.roles[1].limits.turns = Some(40);
    assert_eq!(
        refused(limited),
        Err(BriefRefusal::TeamUnsupported {
            field: TeamField::Turns
        })
    );
    let mut crowded = full_team();
    crowded.roles[0].limits.members = 2;
    assert_eq!(
        refused(crowded),
        Err(BriefRefusal::TeamUnsupported {
            field: TeamField::CoordinatorMembers
        })
    );
}

/// The swarm's own bounds hold for every task item, and the approach is
/// cut to the constraints' 8 KiB rather than refused.
#[test]
fn the_swarms_bounds_hold_for_items_and_the_approach() {
    let wide = PlanItem {
        title: "€".repeat(500),
        ..item("wide", &[], &[])
    };
    assert_eq!(
        brief_of(&task(), &plan_with(vec![wide], None, ""), &at("10:00:00")),
        Err(BriefRefusal::ItemTooLarge { item: slug("wide") })
    );
    let lines = long_lines("é");
    let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
    let heavy = item("heavy", &criteria, &[]);
    assert_eq!(
        brief_of(&task(), &plan_with(vec![heavy], None, ""), &at("10:00:00")),
        Err(BriefRefusal::ItemTooLarge {
            item: slug("heavy")
        })
    );
    let approach = "Spike \"first\".\n".repeat(4000);
    let brief = brief_of(
        &task(),
        &plan_with(items(), None, &approach),
        &at("10:00:00"),
    )
    .unwrap();
    assert!(brief.approach_cut);
    assert!(json_list_bytes(brief.constraints.iter().map(String::as_str)) <= TEXT_MAX_BYTES);
    assert!(approach.starts_with(&brief.constraints[0]));
    assert!(brief.constraints[0].len() > 6000, "the cut keeps what fits");
}

/// Many task items, each within the swarm's bounds, can still make a brief
/// too large for a coordinator to take.
#[test]
fn a_brief_past_64_kib_is_refused() {
    let lines = long_lines("x");
    let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
    let many: Vec<PlanItem> = (0..9)
        .map(|at| item(&format!("item-{at}"), &criteria, &[]))
        .collect();
    match brief_of(&task(), &plan_with(many, None, ""), &at("10:00:00")) {
        Err(BriefRefusal::BriefTooLarge { bytes }) => assert!(bytes > MAX_BRIEF_BYTES),
        other => panic!("{other:?}"),
    }
}

/// The lengths are the swarm board's own: Python's JSON with every
/// character past ASCII escaped.
#[test]
fn text_is_measured_as_the_swarm_board_encodes_it() {
    assert_eq!(json_text_bytes("ab"), 4);
    assert_eq!(json_text_bytes("a\"b\\\n\t"), 2 + 1 + 2 + 1 + 2 + 2 + 2);
    assert_eq!(json_text_bytes("\u{1}\u{7f}é€"), 2 + 4 * 6);
    assert_eq!(json_text_bytes("😀"), 2 + 12);
    assert_eq!(json_list_bytes([]), 2);
    assert_eq!(json_list_bytes(["a", "bc"]), "[\"a\",\"bc\"]".len());
    let criterion = RunCriterion {
        id: "t",
        kind: CriterionKind::Review,
        description: "d",
    };
    let spelt = r#"[{"description":"d","id":"t","kind":"review"},{"description":"d","id":"t","kind":"review"}]"#;
    assert_eq!(criteria_bytes(&[criterion, criterion]), spelt.len());
}

/// Every refusal answers a code a record knows, and names what to change.
#[test]
fn every_refusal_has_a_known_code_and_plain_words() {
    let refusals = [
        (
            BriefRefusal::TeamTooLarge { members: 26 },
            "team_too_large",
            "the team needs 26 members with its coordinator; a swarm holds at most 25",
        ),
        (
            BriefRefusal::TeamUnsupported {
                field: TeamField::Image,
            },
            "team_unsupported",
            "a team's image is not supported yet: leave it out of the plan",
        ),
        (
            BriefRefusal::TeamUnsupported {
                field: TeamField::Turns,
            },
            "team_unsupported",
            "a role's turn limit is not supported yet: leave it out of the plan",
        ),
        (
            BriefRefusal::TeamUnsupported {
                field: TeamField::CoordinatorMembers,
            },
            "team_unsupported",
            "the coordinator role has one member",
        ),
        (
            BriefRefusal::DeadlinePassed,
            "deadline_passed",
            "the team's deadline has passed: propose a plan with a later one",
        ),
        (
            BriefRefusal::ItemTooLarge { item: slug("api") },
            "item_too_large",
            "task item api is too large for a swarm: its title must fit in 1024 bytes and \
             its criteria in 8192",
        ),
        (
            BriefRefusal::BriefTooLarge { bytes: 70_000 },
            "brief_too_large",
            "the brief is 70000 bytes and a coordinator takes at most 65536: shorten the \
             plan's task items",
        ),
    ];
    for (refusal, code, words) in refusals {
        assert_eq!(refusal.code(), code);
        assert_eq!(
            StaticKind::new(code).as_str(),
            code,
            "{code} is a known kind"
        );
        assert_eq!(refusal.to_string(), words);
    }
    for field in [
        TeamField::Image,
        TeamField::Turns,
        TeamField::CoordinatorMembers,
    ] {
        let name = field.as_str();
        assert_eq!(
            StaticKind::new(name).as_str(),
            name,
            "{name} is a known kind"
        );
    }
}
