//! #2481 M4b: a board change is about one task, an allowlist of shapes.
use super::*;
use crate::application::project_board::use_cases::fixtures_tests::{at, fields, person, slug};
use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields, Message};
use crate::domain::project_board::entities::plan::{Plan, PlanFields};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::value_objects::vocabulary::{DiscussionState, PlanState};

fn task(id: &str) -> BoardArtifact {
    BoardArtifact::Task(Task::new(fields(id)).unwrap())
}

fn plan(task: &str, version: u32) -> BoardArtifact {
    BoardArtifact::Plan(
        Plan::new(PlanFields {
            task: slug(task),
            version,
            state: PlanState::Draft,
            approach: String::new(),
            items: vec![],
            team: None,
            handoff: None,
            proposed_by: person("Ada"),
            edited_by: None,
            accepted: None,
            created: at("09:00:00"),
            updated: at("09:00:00"),
        })
        .unwrap(),
    )
}

fn run(task: &str) -> BoardArtifact {
    BoardArtifact::Run(
        Run::new(RunFields {
            task: slug(task),
            number: 1,
            plan_version: 1,
            started: at("09:00:00"),
            ended: None,
            outcome: None,
            cost: None,
            items: vec![],
        })
        .unwrap(),
    )
}

fn discussion() -> BoardArtifact {
    BoardArtifact::Discussion(
        Discussion::new(DiscussionFields {
            id: slug("d"),
            title: "D".into(),
            task: Some(slug("a")),
            state: DiscussionState::Open,
            started_by: person("Ada"),
            messages: vec![Message {
                author: person("Ada"),
                at: at("09:00:00"),
                text: "Why?".into(),
            }],
            created: at("09:00:00"),
            updated: at("09:00:00"),
        })
        .unwrap(),
    )
}

#[test]
fn a_change_writes_its_task_first_or_only_one_tasks_other_artifacts() {
    let taken: [(&str, Vec<BoardArtifact>); 4] = [
        ("a task alone", vec![task("a")]),
        (
            "a task, then its plans",
            vec![task("a"), plan("a", 1), plan("a", 2)],
        ),
        ("a plan alone", vec![plan("a", 1)]),
        ("a plan and a run of one task", vec![plan("a", 2), run("a")]),
    ];
    for (case, artifacts) in taken {
        assert!(one_tasks_change(&artifacts), "{case}");
    }
    let refused: [(&str, Vec<BoardArtifact>); 9] = [
        ("nothing", vec![]),
        ("a task, then another's plan", vec![task("a"), plan("b", 1)]),
        ("two tasks", vec![task("a"), task("b")]),
        ("one task twice", vec![task("a"), task("a")]),
        ("a task, then a discussion", vec![task("a"), discussion()]),
        ("plans of two tasks", vec![plan("a", 1), plan("b", 1)]),
        ("a plan, then its task", vec![plan("a", 1), task("a")]),
        ("a discussion alone", vec![discussion()]),
        ("a run, then a discussion", vec![run("a"), discussion()]),
    ];
    for (case, artifacts) in refused {
        assert!(!one_tasks_change(&artifacts), "{case}");
    }
}

/// The assert stands on the write path: a change of plans of two tasks
/// stops before anything is written.
#[test]
#[should_panic(
    expected = "a board change writes its task first, or only one task's other artifacts"
)]
fn commit_refuses_a_change_of_two_tasks() {
    use crate::application::project_board::use_cases::fixtures_tests::{SetClock, board};
    let store = board();
    let clock = SetClock::at("10:00:00");
    let _ = commit(&store, &*clock, "two", None, |_, _| {
        Ok(vec![plan("a", 1), plan("b", 1)])
    });
}

#[test]
#[should_panic(expected = "a plan's change writes its plan alone")]
fn a_plans_change_is_its_plan_alone() {
    written_plan(vec![plan("a", 1), plan("a", 2)]);
}
