//! The agent tool's `show` within its cap (#2481 M4a review round 1): a
//! plan, runs and versions at the board's limits come back within
//! [`MAX_SHOW_BYTES`], the task whole; what was left out is counted; a
//! show that fits is given as it is.
use serde_json::{Value, json};

use super::{MAX_SHOW_BYTES, capped};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::operations::BoardOutcome;
use crate::application::project_board::ports::{ArtifactKey, BoardHead, QuarantinedFile};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{
    MAX_APPROACH_CHARS, MAX_PLAN_VERSION, Plan, PlanFields, PlanItem,
};
use crate::domain::project_board::entities::run::{MAX_RUN_NUMBER, Run, RunFields, RunItem};
use crate::domain::project_board::entities::task::{MAX_DESCRIPTION_CHARS, Task, TaskFields};
use crate::domain::project_board::entities::task_parts::{MAX_ITEMS, MAX_LINE_CHARS};
use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, PlanState, RunOutcome, TaskKind, TaskStatus,
};
use crate::interface::tools::project_board::answer::outcome;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at() -> Timestamp {
    Timestamp::parse("2026-10-10T09:00:00Z").unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Task `a`, naming plan `accepted`, with `description`.
fn task(description: String, accepted: u32) -> Task {
    Task::new(TaskFields {
        id: slug("a"),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description,
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(accepted),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at(),
        updated: at(),
    })
    .unwrap()
}

/// Version `version` of task `a`'s plan in `state`, with `approach` and
/// `items`.
fn plan(version: u32, state: PlanState, approach: String, items: Vec<PlanItem>) -> Plan {
    let accepted = matches!(state, PlanState::Accepted | PlanState::Superseded);
    Plan::new(PlanFields {
        task: slug("a"),
        version,
        state,
        approach,
        items,
        team: Some(Team {
            roles: vec![Role {
                name: slug("worker"),
                model: "gpt-5.5".into(),
                effort: Effort::High,
                limits: RoleLimits {
                    members: 2,
                    turns: None,
                },
            }],
            budget: None,
            deadline: None,
            image: None,
        }),
        proposed_by: ada(),
        edited_by: None,
        accepted: accepted.then(|| (ada(), at())),
        created: at(),
        updated: at(),
    })
    .unwrap()
}

/// A plan at the board's limits, as JSON's widest: an approach of quotes
/// (each escaped), and every task item with its title and criteria as
/// long as the plan's text budget lets them be.
fn widest_plan(version: u32) -> Plan {
    let approach = "\"".repeat(MAX_APPROACH_CHARS);
    let items = (0..MAX_ITEMS)
        .map(|index| PlanItem {
            id: slug(&format!("item-{index}")),
            title: format!("{index}{}", "\"".repeat(100)),
            criteria: (0..3)
                .map(|n| format!("{n}{}", "\"".repeat(MAX_LINE_CHARS - 1)))
                .collect(),
            depends_on: vec![],
        })
        .collect();
    plan(version, PlanState::Accepted, approach, items)
}

/// Run `number` of plan `version`, ended, with `items` task items, each
/// with an owner and three evidence lines as long as they go.
fn run(number: u32, version: u32, items: usize) -> Run {
    Run::new(RunFields {
        task: slug("a"),
        number,
        plan_version: version,
        started: at(),
        ended: Some(at()),
        outcome: Some(RunOutcome::Failed),
        cost: Some(1),
        items: (0..items)
            .map(|index| RunItem {
                id: slug(&format!("item-{index}")),
                state: ItemState::Done,
                owner: Some(format!("{index}{}", "w".repeat(90))),
                evidence: (0..3)
                    .map(|n| format!("{n}{}", "\"".repeat(MAX_LINE_CHARS - 1)))
                    .collect(),
            })
            .collect(),
    })
    .unwrap()
}

/// A show at the board's limits: the widest plan accepted as the last of
/// every version, every run number with the latest at its widest, and
/// files set aside, each with the longest reason shown.
fn widest(description: String) -> ShownTask {
    let last = MAX_PLAN_VERSION;
    let mut plans: Vec<Plan> = (1..last)
        .map(|version| plan(version, PlanState::Superseded, "x".into(), vec![]))
        .collect();
    plans.push(widest_plan(last));
    let mut runs: Vec<Run> = (1..MAX_RUN_NUMBER).map(|n| run(n, last, 0)).collect();
    runs.push(run(MAX_RUN_NUMBER, last, MAX_ITEMS));
    let set_aside = (1..=200)
        .map(|n| QuarantinedFile {
            path: format!("runs/a.{n}.md"),
            key: Some(ArtifactKey::Run(slug("a"), n)),
            reason: "r".repeat(3_000),
            object: "0".repeat(40),
            carried: true,
        })
        .collect();
    ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task(description, last),
        plans,
        runs,
        set_aside,
    }
}

fn answered(shown: ShownTask) -> Value {
    outcome("show", &BoardOutcome::Shown(shown), None)
}

fn bytes(value: &Value) -> usize {
    value.to_string().len()
}

/// What a list's entries and their count of what was left out add up to.
fn whole(answer: &Value, list: &str, more: &str) -> usize {
    answer[list].as_array().unwrap().len() + answer[more].as_u64().unwrap_or(0) as usize
}

#[test]
fn a_show_at_the_boards_limits_fits_its_cap() {
    let uncut = answered(widest("Why.".into()));
    // Measured: the whole answer the CLI gives, far past any context.
    assert!(bytes(&uncut) > 1_500_000, "{}", bytes(&uncut));
    let cut = capped(uncut.clone());
    assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{}", bytes(&cut));
    assert_eq!(cut["task"], uncut["task"], "the task is whole");
    // The plan's fields stay; its approach and items are cut and counted.
    let (plan, all) = (&cut["plan"], &uncut["plan"]);
    for field in [
        "task",
        "version",
        "state",
        "team",
        "proposed_by",
        "accepted_by",
    ] {
        assert_eq!(plan[field], all[field], "{field}");
    }
    let approach = plan["approach"].as_str().unwrap();
    assert!(all["approach"].as_str().unwrap().starts_with(approach));
    let more = &cut["plan_truncated"];
    assert_eq!(
        approach.chars().count() + more["approach_more"].as_u64().unwrap() as usize,
        MAX_APPROACH_CHARS
    );
    let items = plan["items"].as_array().unwrap();
    assert_eq!(
        items.len() + more["items_more"].as_u64().unwrap() as usize,
        MAX_ITEMS
    );
    assert_eq!(items[..], all["items"].as_array().unwrap()[..items.len()]);
    // The newest runs and versions, in their order, counted.
    let runs = cut["runs"].as_array().unwrap();
    assert_eq!(runs.last().unwrap()["number"], json!(MAX_RUN_NUMBER));
    assert_eq!(whole(&cut, "runs", "runs_more"), MAX_RUN_NUMBER as usize);
    let latest_items = runs.last().unwrap()["items"].as_array().unwrap().len();
    assert_eq!(
        latest_items + cut["latest_run_items_more"].as_u64().unwrap_or(0) as usize,
        MAX_ITEMS
    );
    let numbers: Vec<u64> = runs.iter().map(|r| r["number"].as_u64().unwrap()).collect();
    assert!(numbers.windows(2).all(|pair| pair[0] + 1 == pair[1]));
    assert_eq!(
        whole(&cut, "plans", "plans_more"),
        MAX_PLAN_VERSION as usize
    );
    assert_eq!(whole(&cut, "quarantined", "quarantined_more"), 200);
    assert_eq!(cut["may_be_incomplete"], json!(true));
}

/// A task is never cut: with its longest description, which takes the
/// room alone, the rest is counted and not shown.
#[test]
fn the_task_is_whole_and_the_rest_counted() {
    let description = "\"".repeat(MAX_DESCRIPTION_CHARS);
    let cut = capped(answered(widest(description)));
    assert_eq!(
        cut["task"]["description"].as_str().unwrap().chars().count(),
        MAX_DESCRIPTION_CHARS
    );
    assert_eq!(cut["plan"], json!(null));
    assert_eq!(
        cut["plan_truncated"],
        json!({"approach_more": MAX_APPROACH_CHARS, "items_more": MAX_ITEMS})
    );
    assert_eq!(cut["runs"], json!([]));
    assert_eq!(cut["runs_more"], json!(MAX_RUN_NUMBER));
    assert_eq!(cut["plans_more"], json!(MAX_PLAN_VERSION));
    assert_eq!(cut["quarantined_more"], json!(200));
    // The task and the answer's own fields, nothing more.
    let task = bytes(&cut["task"]);
    assert!(bytes(&cut) - task < 1_024, "{}", bytes(&cut) - task);
}

/// A show that fits is given exactly as the board answered it.
#[test]
fn a_show_that_fits_is_given_as_it_is() {
    let small = ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task("Why.".into(), 1),
        plans: vec![plan(1, PlanState::Accepted, "Build it.".into(), vec![])],
        runs: vec![run(1, 1, 2), run(2, 1, 2)],
        set_aside: vec![],
    };
    let answer = answered(small);
    assert_eq!(capped(answer.clone()), answer);
}

/// A plan that fits stays whole, even when its runs do not all fit.
#[test]
fn a_plan_that_fits_stays_whole_before_the_runs() {
    let mut shown = widest("Why.".into());
    let fits = plan(
        MAX_PLAN_VERSION,
        PlanState::Accepted,
        "Build it.".into(),
        vec![],
    );
    *shown.plans.last_mut().unwrap() = fits;
    let uncut = answered(shown);
    let cut = capped(uncut.clone());
    assert_eq!(cut["plan"], uncut["plan"]);
    assert_eq!(cut.get("plan_truncated"), None);
    assert!(cut["runs_more"].as_u64().unwrap() > 0);
}

/// The tool caps every answered show it gives; a refused one, and any
/// other op's answer but a listing, pass as they are.
#[test]
fn the_tool_caps_every_answered_show() {
    let uncut = answered(widest("Why.".into()));
    assert!(bytes(&super::super::summarised(uncut)) <= MAX_SHOW_BYTES);
    let refused = json!({"ok": false, "op": "show", "error": {"code": "not_found"}});
    assert_eq!(super::super::summarised(refused.clone()), refused);
}
