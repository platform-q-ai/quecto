//! #2705 T5: `show` gives each item of its plan a state from the task's
//! latest run (`not_started` with none), and names the task's states that
//! contradict its runs and plan as `inconsistent`, rewriting nothing.
use super::*;
use crate::domain::project_board::value_objects::vocabulary::Resolution;

/// Task `a` as `change` leaves it.
fn task_as(change: impl FnOnce(&mut TaskFields)) -> Task {
    let mut fields = task().into_fields();
    change(&mut fields);
    Task::new(fields).unwrap()
}

fn answered_task(task: Task, plans: Vec<Plan>, runs: Vec<Run>) -> Value {
    answered(ShownTask {
        task,
        ..shown(plans, runs, vec![])
    })
}

#[test]
fn items_with_no_run_read_not_started() {
    let answer = answered(shown(vec![plan(2, PlanState::Accepted)], vec![], vec![]));
    assert_eq!(answer["plan"]["items"][0]["state"], json!("not_started"));
    assert_eq!(answer["inconsistent"], json!([]));
}

#[test]
fn items_read_their_state_from_the_latest_run() {
    let answer = answered(shown(
        vec![plan(2, PlanState::Accepted)],
        vec![run(1, 2, RunOutcome::Failed, &[])],
        vec![],
    ));
    assert_eq!(answer["plan"]["items"][0]["state"], json!("done"));
}

/// The land-layout-2652 case: done, no plan accepted, no run.
#[test]
fn a_done_task_with_no_plan_and_no_run_reads_inconsistent_done_without_plan() {
    let done = task_as(|fields| {
        fields.status = TaskStatus::Done;
        fields.resolution = Some(Resolution::Completed);
        fields.accepted_plan = None;
    });
    let answer = answered_task(done.clone(), vec![], vec![]);
    assert_eq!(answer["plan"], json!(null));
    assert_eq!(answer["runs"], json!([]));
    assert_eq!(answer["inconsistent"], json!(["done_without_plan"]));
    assert_eq!(answer["task"], task_json(&done), "the task as it is");
}

/// While a file that may hold one of its runs is set aside, the task's
/// runs may be incomplete: nothing is judged from them.
#[test]
fn a_task_whose_runs_may_be_incomplete_is_not_judged() {
    let done = task_as(|fields| {
        fields.status = TaskStatus::Done;
        fields.resolution = Some(Resolution::Completed);
        fields.accepted_plan = None;
    });
    let set_aside = QuarantinedFile {
        key: Some(ArtifactKey::Run(slug("a"), 1)),
        path: "runs/a.1.md".into(),
        reason: "malformed".into(),
        object: "b".repeat(40),
        carried: true,
    };
    let answer = answered(ShownTask {
        task: done,
        ..shown(vec![], vec![], vec![set_aside])
    });
    assert_eq!(answer["inconsistent"], json!([]));
}
