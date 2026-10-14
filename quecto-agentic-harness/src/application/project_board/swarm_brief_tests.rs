use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::telemetry::StaticKind;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

/// The fixture task, ready, its plan v1 accepted.
fn task() -> Task {
    Task::new(TaskFields {
        id: slug("board-store"),
        title: "Board store".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
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

/// Accepted version 1 of the task's plan.
fn plan_with(items: Vec<PlanItem>, team: Option<Team>, approach: &str) -> Plan {
    Plan::new(PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Accepted,
        approach: approach.into(),
        items,
        team,
        proposed_by: person("Ada"),
        edited_by: None,
        accepted: Some((person("Ada"), at("08:30:00"))),
        created: at("08:30:00"),
        updated: at("08:30:00"),
    })
    .unwrap()
}

/// `plan`'s brief, the task started by Bob at 10:00.
fn brief(plan: &Plan) -> Result<SwarmBrief, BriefRefusal> {
    let now = at("10:00:00");
    let started = start(&task(), &[], Some(plan), &person("Bob"), &now).unwrap();
    brief_of(&started, &now)
}

fn team(roles: Vec<Role>) -> Team {
    Team {
        roles,
        budget: Some(Budget { tokens: 2_000_000 }),
        deadline: Some(at("16:00:00")),
        image: None,
    }
}

/// `api` listed before the `schema` it depends on.
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
        role("coordinator", "anthropic/claude-opus", 1),
        role("builder", "gpt-5.5", 2),
        role("reviewer", "glm-5", 1),
    ])
}

fn with_team(team: Team) -> Result<SwarmBrief, BriefRefusal> {
    brief(&plan_with(items(), Some(team), ""))
}

const POINTER: &str = "The full plan is on the project board (task board-store, plan v1); \
                       ask your parent for any part cut here.";

/// The whole mapping of a plan, as §3.2 of the shortcut plan lays it out.
#[test]
fn a_plan_maps_onto_the_swarms_inputs() {
    let got = brief(&plan_with(items(), Some(full_team()), "Spike, then build.")).unwrap();
    let want = SwarmBrief {
        task: slug("board-store"),
        plan_version: 1,
        goal: "Board task board-store, plan v1: Board store".into(),
        constraints: vec!["Spike, then build.".into(), POINTER.into()],
        approach_cut: false,
        criteria: RUN_CRITERIA.to_vec(),
        member_limit: Some(4),
        deadline_in_seconds: 6 * 3600,
        deadline_clamped: false,
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
            model: "anthropic/claude-opus".into(),
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
    assert_eq!(got, want);
    // Each text and number a line (its UTF-8 bytes and one), counted by
    // hand: the template 4096; the task 12; the goal 45; the constraints
    // 19 + 106; the criteria 11 + 7 + 76 and 16 + 7 + 100; the items 3 x 64
    // for the template, then schema 25 + 17, api 19 + 18 + 31 + 7 (its
    // dependency), docs 21 + 18 + 4 + 7; the coordinator 12 + 22 + 5; the
    // workers 8 + 8 + 5 and 9 + 6 + 5; the numbers v1 2, limit 4 2,
    // deadline 21600 6 and budget 2000000 8.
    assert_eq!(got.bytes(), 4952);
}

/// No team: the coordinator picks its pool, its model is the launcher's,
/// and the run has a day.
#[test]
fn a_plan_without_a_team_leaves_the_pool_to_the_coordinator() {
    let got = brief(&plan_with(items(), None, "")).unwrap();
    assert_eq!(got.member_limit, None);
    assert_eq!(got.deadline_in_seconds, 24 * 3600);
    assert!(!got.deadline_clamped);
    assert_eq!(got.usage_budget, None);
    assert_eq!(got.coordinator, None);
    assert!(got.not_applied.is_empty());
    // A blank approach is no constraint; the pointer to the plan stays.
    assert_eq!(got.constraints, [POINTER]);
}

/// The swarm holds at most 25 members, its coordinator one of them, and
/// a team names at least one worker role.
#[test]
fn the_member_limit_is_the_coordinator_and_every_worker() {
    let limit = |workers: &[u16]| {
        let roles = workers
            .iter()
            .enumerate()
            .map(|(at, members)| role(&format!("pool-{at}"), "gpt-5.5", *members))
            .collect();
        with_team(team(roles)).map(|brief| brief.member_limit)
    };
    assert_eq!(limit(&[1]), Ok(Some(2)));
    assert_eq!(limit(&[12, 12]), Ok(Some(25)));
    let refused = |members| Err(BriefRefusal::TeamTooLarge { members });
    assert_eq!(limit(&[12, 13]), refused(26));
    assert_eq!(limit(&[32, 32]), refused(65));
    let alone = team(vec![role("coordinator", "anthropic/claude-opus", 1)]);
    assert_eq!(
        with_team(alone),
        Err(BriefRefusal::TeamUnsupported {
            field: TeamField::Workers
        })
    );
}

/// A deadline is the plan's less now: at least ten minutes off, and at
/// most seven days, a later one clamped and said to be.
#[test]
fn the_deadline_is_ten_minutes_to_a_week_off() {
    let deadline = |deadline: &str| {
        let mut with = full_team();
        with.deadline = Some(Timestamp::parse(deadline).unwrap());
        with_team(with).map(|brief| (brief.deadline_in_seconds, brief.deadline_clamped))
    };
    assert_eq!(deadline("2026-10-10T10:10:00Z"), Ok((600, false)));
    assert_eq!(deadline("2026-10-17T10:00:00Z"), Ok((7 * 24 * 3600, false)));
    assert_eq!(deadline("2026-10-17T10:00:01Z"), Ok((7 * 24 * 3600, true)));
    assert_eq!(deadline("2027-01-01T00:00:00Z"), Ok((7 * 24 * 3600, true)));
    for soon in ["2026-10-10T10:09:59Z", "2026-10-10T10:00:01Z"] {
        assert_eq!(deadline(soon), Err(BriefRefusal::DeadlineTooSoon), "{soon}");
    }
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
    let unsupported = |field| Err(BriefRefusal::TeamUnsupported { field });
    let mut imaged = full_team();
    imaged.image = Some("ghcr.io/platform-q-ai/quecto:1".into());
    assert_eq!(with_team(imaged), unsupported(TeamField::Image));
    for limited in [0, 1] {
        let mut turns = full_team();
        turns.roles[limited].limits.turns = Some(40);
        assert_eq!(with_team(turns), unsupported(TeamField::Turns));
    }
    let mut crowded = full_team();
    crowded.roles[0].limits.members = 2;
    assert_eq!(
        with_team(crowded),
        unsupported(TeamField::CoordinatorMembers)
    );
}

/// A plan with nothing to do starts nothing.
#[test]
fn a_plan_without_task_items_is_refused() {
    assert_eq!(
        brief(&plan_with(vec![], None, "")),
        Err(BriefRefusal::NoTaskItems)
    );
}

/// A task item's title (with its prefix) takes at most 1024 bytes, and
/// its acceptance at most 8 KiB once encoded, as the swarm takes them.
#[test]
fn every_task_item_fits_the_swarms_bounds() {
    // `wide: ` and 1018 bytes: 339 three-byte characters and one more.
    let titled = |tail: &str| {
        let wide = PlanItem {
            title: format!("{}{tail}", "€".repeat(339)),
            ..item("wide", &[], &[])
        };
        brief(&plan_with(vec![wide], None, ""))
    };
    let fits = titled("x").unwrap();
    assert_eq!(fits.items[0].title.len(), 1024);
    assert_eq!(
        titled("xx"),
        Err(BriefRefusal::ItemTooLarge {
            item: slug("wide"),
            limit: ItemLimit::Title,
            bytes: 1025
        })
    );
    let lines = long_lines("é");
    let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
    let heavy = item("heavy", &criteria, &[]);
    let encoded = json_list_bytes(criteria.iter().copied());
    assert!(encoded > TEXT_MAX_BYTES);
    assert_eq!(
        brief(&plan_with(vec![heavy], None, "")),
        Err(BriefRefusal::ItemTooLarge {
            item: slug("heavy"),
            limit: ItemLimit::Criteria,
            bytes: encoded
        })
    );
}

/// An approach is trimmed, then kept whole when it fits; otherwise as
/// much of it as fits, marked where it was cut.
#[test]
fn the_approach_is_trimmed_and_cut_to_the_most_that_fits() {
    let padded = format!("{}Real text.\n\n", " ".repeat(9000));
    let got = brief(&plan_with(items(), None, &padded)).unwrap();
    assert_eq!(got.constraints, ["Real text.", POINTER]);
    assert!(!got.approach_cut);
    let approach = "Spike \"first\".\n".repeat(4000);
    let got = brief(&plan_with(items(), None, &approach)).unwrap();
    assert!(got.approach_cut);
    let encoded = |constraints: &[String]| json_list_bytes(constraints.iter().map(String::as_str));
    assert!(encoded(&got.constraints) <= TEXT_MAX_BYTES);
    let kept = got.constraints[0]
        .strip_suffix(CUT_MARKER)
        .expect("a cut approach ends with the marker");
    assert!(approach.starts_with(kept));
    // One more character would not have fitted.
    let next = approach[kept.len()..].chars().next().unwrap();
    let longer = vec![format!("{kept}{next}{CUT_MARKER}"), POINTER.to_owned()];
    assert!(encoded(&longer) > TEXT_MAX_BYTES);
}

/// Many task items, each within the swarm's bounds, can still make a brief
/// too large for a coordinator to take.
#[test]
fn a_brief_past_64_kib_is_refused_with_its_size() {
    let lines = long_lines("x");
    let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
    let many: Vec<PlanItem> = (0..9)
        .map(|at| item(&format!("item-{at}"), &criteria, &[]))
        .collect();
    match brief(&plan_with(many, None, "")) {
        Err(BriefRefusal::BriefTooLarge { bytes }) => {
            assert!(bytes > MAX_BRIEF_BYTES);
            assert!(bytes > 9 * 16 * 500, "the criteria alone are counted");
        }
        other => panic!("{other:?}"),
    }
}

/// The size counts everything a coordinator reads: role names, models,
/// efforts and numbers too, and the template's own room.
#[test]
fn the_size_counts_roles_models_and_numbers() {
    let size = |team: Team| with_team(team).unwrap().bytes();
    let base = size(full_team());
    let mut longer = full_team();
    longer.roles[1].model = "gpt-5.5-long".into();
    assert_eq!(size(longer), base + 5);
    let mut renamed = full_team();
    renamed.roles[2].name = slug("reviewers");
    assert_eq!(size(renamed), base + 1);
    let mut coordinated = full_team();
    coordinated.roles[0].model = "anthropic/claude-opus-x".into();
    assert_eq!(size(coordinated), base + 2);
    let mut funded = full_team();
    funded.budget = Some(Budget { tokens: 20_000_000 });
    assert_eq!(size(funded), base + 1);
    let none = brief(&plan_with(vec![item("a", &[], &[])], None, "")).unwrap();
    assert!(none.bytes() > TEMPLATE_BYTES + TEMPLATE_ITEM_BYTES);
}

/// The lengths are the swarm board's own: Python's JSON with every
/// character past ASCII escaped.
#[test]
fn text_is_measured_as_the_swarm_board_encodes_it() {
    assert_eq!(json_list_bytes([]), 2);
    assert_eq!(json_list_bytes(["a", "bc"]), "[\"a\",\"bc\"]".len());
    assert_eq!(json_list_bytes(["é"]), "[\"\\u00e9\"]".len());
}

/// Every refusal answers a code a record knows, and names what to change.
#[test]
fn every_refusal_has_a_known_code_and_plain_words() {
    let unsupported = |field| BriefRefusal::TeamUnsupported { field };
    let refusals = [
        (
            BriefRefusal::TeamTooLarge { members: 26 },
            "team_too_large",
            "the team needs 26 members with its coordinator; a swarm holds at most 25",
        ),
        (
            unsupported(TeamField::Image),
            "team_unsupported",
            "a team's image is not supported yet: leave it out of the plan",
        ),
        (
            unsupported(TeamField::Turns),
            "team_unsupported",
            "a role's turn limit is not supported yet: leave it out of the plan",
        ),
        (
            unsupported(TeamField::CoordinatorMembers),
            "team_unsupported",
            "a team's coordinator role must have exactly one member",
        ),
        (
            unsupported(TeamField::Workers),
            "team_unsupported",
            "a team needs at least one worker role besides the coordinator",
        ),
        (
            BriefRefusal::DeadlinePassed,
            "deadline_passed",
            "the team's deadline has passed: propose a plan with a later one",
        ),
        (
            BriefRefusal::DeadlineTooSoon,
            "deadline_too_soon",
            "the team's deadline is too soon: propose a plan with a deadline at least 10 \
             minutes off",
        ),
        (
            BriefRefusal::NoTaskItems,
            "no_task_items",
            "the plan has no task items: add at least one before starting",
        ),
        (
            BriefRefusal::ItemTooLarge {
                item: slug("api"),
                limit: ItemLimit::Title,
                bytes: 1030,
            },
            "item_too_large",
            "task item api's title is 1030 bytes with its prefix; a swarm takes at most 1024",
        ),
        (
            BriefRefusal::ItemTooLarge {
                item: slug("api"),
                limit: ItemLimit::Criteria,
                bytes: 9000,
            },
            "item_too_large",
            "task item api's criteria are 9000 bytes once encoded; a swarm takes at most 8192",
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
    let fields = [
        TeamField::Image,
        TeamField::Turns,
        TeamField::CoordinatorMembers,
        TeamField::Workers,
    ];
    let names = fields.iter().map(|field| field.as_str());
    let limits = [ItemLimit::Title, ItemLimit::Criteria].map(ItemLimit::as_str);
    for name in names.chain(limits) {
        assert_eq!(
            StaticKind::new(name).as_str(),
            name,
            "{name} is a known kind"
        );
    }
}

/// Each bound holds at its limit exactly: criteria of exactly 8192 bytes
/// once encoded are taken, a byte more refused.
#[test]
fn criteria_of_exactly_8_kib_are_taken() {
    // 15 lines of 500 ASCII characters (502 bytes each, quoted), and one of
    // `15 `, `é` x 100 (6 bytes each) and `x` x `tail`: with the list's
    // brackets and commas, 2 + 15 x 502 + (2 + 3 + 600 + tail) + 15.
    let criteria_of = |tail: usize| {
        let mut lines = long_lines("x");
        lines[15] = format!("15 {}{}", "é".repeat(100), "x".repeat(tail));
        lines
    };
    let exact = criteria_of(40);
    assert_eq!(
        json_list_bytes(exact.iter().map(String::as_str)),
        TEXT_MAX_BYTES
    );
    let planned = |lines: &[String]| {
        let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
        brief(&plan_with(vec![item("exact", &criteria, &[])], None, ""))
    };
    assert!(planned(&exact).is_ok());
    assert_eq!(
        planned(&criteria_of(41)),
        Err(BriefRefusal::ItemTooLarge {
            item: slug("exact"),
            limit: ItemLimit::Criteria,
            bytes: TEXT_MAX_BYTES + 1
        })
    );
}

/// An approach that exactly fits the constraints' 8 KiB is kept whole; a
/// character more is cut.
#[test]
fn an_approach_that_exactly_fits_is_not_cut() {
    // The list's brackets and comma, the approach's quotes, the pointer.
    let room = TEXT_MAX_BYTES - 2 - 1 - 2 - json_text(POINTER).len();
    let exact = "x".repeat(room);
    let got = brief(&plan_with(items(), None, &exact)).unwrap();
    assert_eq!(got.constraints, [exact.as_str(), POINTER]);
    assert!(!got.approach_cut);
    let over = "x".repeat(room + 1);
    assert!(
        brief(&plan_with(items(), None, &over))
            .unwrap()
            .approach_cut
    );
}

/// A brief of exactly [`MAX_BRIEF_BYTES`] is taken; a byte more refused.
#[test]
fn a_brief_of_exactly_64_kib_is_taken() {
    let lines = long_lines("x");
    let criteria: Vec<&str> = lines.iter().map(String::as_str).collect();
    let heavy: Vec<PlanItem> = (0..7)
        .map(|at| item(&format!("item-{at}"), &criteria, &[]))
        .collect();
    let sized = |approach: usize| brief(&plan_with(heavy.clone(), None, &"x".repeat(approach)));
    // A one-character approach is one line of two bytes; each more
    // character is a byte more, while the approach is kept whole.
    let base = sized(1).unwrap().bytes();
    let short = MAX_BRIEF_BYTES - base;
    assert!((1..7000).contains(&short), "{short}");
    let exact = sized(1 + short).unwrap();
    assert!(!exact.approach_cut);
    assert_eq!(exact.bytes(), MAX_BRIEF_BYTES);
    assert_eq!(
        sized(2 + short),
        Err(BriefRefusal::BriefTooLarge {
            bytes: MAX_BRIEF_BYTES + 1
        })
    );
}

/// S4c review round 1 (M2): the coordinator's model must name its
/// provider, as spawn takes it; none is guessed, and a worker role's model
/// (not applied) is not judged.
#[test]
fn the_coordinators_model_must_name_its_provider() {
    let unqualified = team(vec![
        role("coordinator", "claude-opus", 1),
        role("builder", "gpt-5.5", 2),
    ]);
    assert_eq!(with_team(unqualified), Err(BriefRefusal::CoordinatorModel));
    let qualified = team(vec![
        role("coordinator", "anthropic/claude-opus", 1),
        role("builder", "gpt-5.5", 2),
    ]);
    let brief = with_team(qualified).unwrap();
    assert_eq!(
        brief.coordinator.map(|choice| choice.model),
        Some("anthropic/claude-opus".to_string())
    );
    assert_eq!(BriefRefusal::CoordinatorModel.code(), "coordinator_model");
}
