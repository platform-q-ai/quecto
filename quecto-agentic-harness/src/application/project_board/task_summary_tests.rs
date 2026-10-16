//! A listed task's summary: the newest draft or accepted plan, the latest
//! readable run with its deadline and task items counted by state, and
//! whether a file set aside may hold one of its plans or runs.
use super::{ItemCounts, PlanSummary, RunSummary, TaskSummary, summary_of};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::ports::{ArtifactKey, BoardHead, QuarantinedFile};
use crate::application::project_board::swarm_brief::DEFAULT_DEADLINE_SECONDS;
use crate::application::project_board::use_cases::fixtures_tests::{
    at, fields, plan_item, plan_of, slug,
};
use crate::domain::project_board::entities::plan::{Plan, PlanFields};
use crate::domain::project_board::entities::run::{Run, RunFields, RunItem};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, PlanState, RunOutcome};

/// Task `a` with `plans` and `runs`, nothing set aside.
fn shown(plans: Vec<Plan>, runs: Vec<Run>) -> ShownTask {
    ShownTask {
        head: BoardHead("h".into()),
        task: Task::new(fields("a")).unwrap(),
        plans,
        runs,
        set_aside: vec![],
    }
}

/// Task `a`'s plan `version` in `state` (two items, `schema` and `api`,
/// and a team naming no deadline).
fn plan(version: u32, state: PlanState) -> Plan {
    plan_of("a", version, state)
}

/// `plan` with its team's deadline `seconds` (none: no team at all).
fn with_deadline(plan: Plan, seconds: Option<u32>) -> Plan {
    let base = plan.into_fields();
    let team = match seconds {
        Some(seconds) => base.team.clone().map(|team| Team {
            deadline_seconds: Some(seconds),
            ..team
        }),
        None => None,
    };
    Plan::new(PlanFields { team, ..base }).unwrap()
}

/// Run `number` of task `a`'s plan `version`, started at 10:00, running.
fn running(number: u32, version: u32) -> Run {
    Run::new(RunFields {
        task: slug("a"),
        number,
        plan_version: version,
        started: at("10:00:00"),
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
    })
    .unwrap()
}

/// `run` ended at 11:00 as `outcome`, its task items in `states`, each an
/// item of the plan by id.
fn ended(run: Run, outcome: RunOutcome, states: &[(&str, ItemState)]) -> Run {
    let items = states
        .iter()
        .map(|(id, state)| RunItem {
            id: slug(id),
            state: *state,
            owner: None,
            evidence: vec![],
        })
        .collect();
    Run::new(RunFields {
        ended: Some(at("11:00:00")),
        outcome: Some(outcome),
        cost: Some(10),
        items,
        ..run.into_fields()
    })
    .unwrap()
}

fn plan_summary(version: u32, state: PlanState) -> Option<PlanSummary> {
    Some(PlanSummary { version, state })
}

fn run_of(summary: &TaskSummary) -> &RunSummary {
    summary.run.as_ref().expect("a run is summarised")
}

#[test]
fn a_task_with_no_plan_or_run_summarises_none() {
    assert_eq!(
        summary_of(&shown(vec![], vec![])),
        TaskSummary {
            plan: None,
            run: None,
            may_be_incomplete: false,
        }
    );
}

#[test]
fn a_draft_alone_is_the_plan() {
    let summary = summary_of(&shown(vec![plan(1, PlanState::Draft)], vec![]));
    assert_eq!(summary.plan, plan_summary(1, PlanState::Draft));
    assert_eq!(summary.run, None);
}

#[test]
fn a_draft_newer_than_the_accepted_plan_wins() {
    let plans = vec![plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let summary = summary_of(&shown(plans, vec![]));
    assert_eq!(summary.plan, plan_summary(2, PlanState::Draft));
    // An accepted plan newer than every draft is the plan.
    let plans = vec![plan(1, PlanState::Discarded), plan(2, PlanState::Accepted)];
    let summary = summary_of(&shown(plans, vec![]));
    assert_eq!(summary.plan, plan_summary(2, PlanState::Accepted));
}

#[test]
fn superseded_and_discarded_plans_are_skipped() {
    let plans = vec![
        plan(1, PlanState::Accepted),
        plan(2, PlanState::Superseded),
        plan(3, PlanState::Discarded),
    ];
    let summary = summary_of(&shown(plans, vec![]));
    assert_eq!(summary.plan, plan_summary(1, PlanState::Accepted));
    // Only versions no summary names: none.
    let plans = vec![
        plan(1, PlanState::Superseded),
        plan(2, PlanState::Discarded),
    ];
    assert_eq!(summary_of(&shown(plans, vec![])).plan, None);
}

/// A running run has no outcome, no end and no task items yet: a run
/// records them when it ends.
#[test]
fn a_running_run_is_summarised_with_its_deadline() {
    let plans = vec![plan(1, PlanState::Accepted)];
    let summary = summary_of(&shown(plans, vec![running(1, 1)]));
    assert_eq!(
        run_of(&summary),
        &RunSummary {
            number: 1,
            plan_version: 1,
            outcome: None,
            started: at("10:00:00"),
            ended: None,
            deadline: Some(Timestamp::parse("2026-10-11T10:00:00Z").unwrap()),
            items: ItemCounts::default(),
        }
    );
}

#[test]
fn an_ended_runs_items_are_counted_by_state() {
    let ids = ["one", "two", "three", "four", "five", "six"];
    let base = plan(1, PlanState::Accepted).into_fields();
    let five = Plan::new(PlanFields {
        items: ids.iter().map(|id| plan_item(id, &[])).collect(),
        ..base
    })
    .unwrap();
    let states = [
        ("one", ItemState::Todo),
        ("two", ItemState::Doing),
        ("three", ItemState::Doing),
        ("four", ItemState::Review),
        ("five", ItemState::Done),
        ("six", ItemState::Blocked),
    ];
    let run = ended(running(1, 1), RunOutcome::Failed, &states);
    let summary = summary_of(&shown(vec![five], vec![run]));
    assert_eq!(
        run_of(&summary).items,
        ItemCounts {
            total: 6,
            todo: 1,
            doing: 2,
            review: 1,
            done: 1,
            blocked: 1,
        }
    );
}

#[test]
fn each_terminal_state_is_its_outcome_with_its_end() {
    let items = [("schema", ItemState::Done), ("api", ItemState::Done)];
    for outcome in RunOutcome::ALL.iter().copied() {
        let run = ended(running(1, 1), outcome, &items);
        let summary = summary_of(&shown(vec![plan(1, PlanState::Accepted)], vec![run]));
        let run = run_of(&summary);
        assert_eq!(run.outcome, Some(outcome));
        assert_eq!(run.ended, Some(at("11:00:00")));
        assert_eq!(run.items.total, 2);
        assert_eq!(run.items.done, 2);
    }
    assert_eq!(RunOutcome::ALL.len(), 4, "succeeded, failed, stopped, lost");
}

/// The latest readable run is summarised, of whichever plan it ran.
#[test]
fn the_latest_run_is_summarised() {
    let plans = vec![plan(1, PlanState::Superseded), plan(2, PlanState::Accepted)];
    let first = ended(running(1, 1), RunOutcome::Stopped, &[]);
    let summary = summary_of(&shown(plans, vec![first, running(2, 2)]));
    let run = run_of(&summary);
    assert_eq!((run.number, run.plan_version, run.outcome), (2, 2, None));
}

#[test]
fn the_deadline_is_the_teams_or_a_day() {
    let named = with_deadline(plan(1, PlanState::Accepted), Some(3_600));
    let summary = summary_of(&shown(vec![named], vec![running(1, 1)]));
    assert_eq!(
        run_of(&summary).deadline,
        Some(Timestamp::parse("2026-10-10T11:00:00Z").unwrap())
    );
    // A team naming no deadline, and no team at all: a day.
    let day = at("10:00:00")
        .plus_seconds(DEFAULT_DEADLINE_SECONDS)
        .unwrap();
    for plan in [
        plan(1, PlanState::Accepted),
        with_deadline(plan(1, PlanState::Accepted), None),
    ] {
        let summary = summary_of(&shown(vec![plan], vec![running(1, 1)]));
        assert_eq!(run_of(&summary).deadline, Some(day.clone()));
    }
}

/// A deadline is the plan's the run ran: of its version, never a draft
/// newer than it; none while that plan is not readable.
#[test]
fn the_deadline_is_the_plan_the_run_ran() {
    let plans = vec![
        with_deadline(plan(1, PlanState::Accepted), Some(3_600)),
        with_deadline(plan(2, PlanState::Draft), Some(7_200)),
    ];
    let summary = summary_of(&shown(plans, vec![running(1, 1)]));
    assert_eq!(
        run_of(&summary).deadline,
        Some(Timestamp::parse("2026-10-10T11:00:00Z").unwrap())
    );
    let missing = summary_of(&shown(
        vec![plan(2, PlanState::Accepted)],
        vec![running(1, 1)],
    ));
    assert_eq!(run_of(&missing).deadline, None);
}

#[test]
fn a_set_aside_file_may_leave_it_incomplete() {
    let mut task = shown(vec![plan(1, PlanState::Accepted)], vec![]);
    task.set_aside = vec![QuarantinedFile {
        path: "runs/a.1.md".into(),
        key: Some(ArtifactKey::Run(slug("a"), 1)),
        reason: "malformed".into(),
        object: "b".repeat(40),
        carried: true,
    }];
    let summary = summary_of(&task);
    assert!(summary.may_be_incomplete);
    assert_eq!(summary.may_be_incomplete, task.may_be_incomplete());
    assert_eq!(summary.plan, plan_summary(1, PlanState::Accepted));
    assert_eq!(summary.run, None, "only a readable run is summarised");
}
