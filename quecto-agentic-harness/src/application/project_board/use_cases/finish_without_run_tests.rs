//! #2705 T4: a task done without a run, through its use case. Its reason
//! is normalised as it enters; a task linking pull requests is done so
//! only once every one reads merged on the host (the owner's choice, T4
//! review round 1); and a task the rule refuses anyway never has its pull
//! requests read.
use super::FinishWithoutRun;
use crate::application::project_board::dto::{NoPullRequestHost, ProjectBoardError};
use crate::application::project_board::ports::{
    ProjectBoardStore, PullRequestState, PullRequestStates, PullRequestStatesError,
};
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{Resolution, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::test_support::memory_pull_requests::MemoryPullRequests;
use std::sync::Arc;

/// The board holding `tasks`, each stamped 08:00 to 09:10.
fn seeded(tasks: Vec<TaskFields>) -> MemoryBoardStore {
    let store = board();
    let tasks: Vec<Task> = tasks
        .into_iter()
        .map(|fields| {
            Task::new(TaskFields {
                created: at("08:00:00"),
                updated: at("09:10:00"),
                ..fields
            })
            .unwrap()
        })
        .collect();
    let head = store.read().unwrap().head;
    store.write_tasks(&head, &tasks, "seed").unwrap();
    store
}

/// Task `a`, claimed by Ada, linking `prs`.
fn claimed(prs: &[u64]) -> TaskFields {
    TaskFields {
        status: TaskStatus::Claimed,
        claim: Some(Claim {
            holder: person("Ada"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        prs: prs.to_vec(),
        ..fields("a")
    }
}

/// The use case over `store`, as Ada, reading `host`.
fn finish(
    store: &MemoryBoardStore,
    host: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
) -> FinishWithoutRun {
    FinishWithoutRun::reading(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:00:00"),
        host,
    )
    .unwrap()
}

fn current(store: &MemoryBoardStore) -> TaskFields {
    let board = store.read().unwrap();
    let task = board
        .tasks
        .iter()
        .find(|task| task.fields().id == slug("a"));
    task.unwrap().fields().clone()
}

fn reading(host: &MemoryPullRequests) -> Result<Arc<dyn PullRequestStates>, NoPullRequestHost> {
    Ok(Arc::new(host.clone()))
}

#[test]
fn a_task_done_without_a_run_keeps_its_resolution_and_normalised_reason() {
    let store = seeded(vec![fields("a")]);
    let host = Err(NoPullRequestHost::NotAPullRequestHost);
    let done = finish(&store, host)
        .execute_noted(&slug("a"), "Answered in chat:\u{a0}no change", None)
        .unwrap();
    assert_eq!(done.fields(), &current(&store), "the task as written");
    let fields = current(&store);
    assert_eq!(
        (fields.status, fields.resolution, fields.reason.as_deref()),
        (
            TaskStatus::Done,
            Some(Resolution::DoneWithoutRun),
            Some("Answered in chat: no change")
        ),
        "no pull request linked: no host asked"
    );
}

#[test]
fn done_without_run_without_a_reason_is_refused() {
    let store = seeded(vec![fields("a")]);
    let before = current(&store);
    let refused = finish(&store, Err(NoPullRequestHost::NotAPullRequestHost)).execute_noted(
        &slug("a"),
        " \t ",
        None,
    );
    match refused {
        Err(ProjectBoardError::Rule(BoardRuleError::Invalid(error))) => {
            assert_eq!(error.field, "reason");
        }
        other => panic!("no reason is refused: {other:?}"),
    }
    assert_eq!(current(&store), before, "nothing is written");
}

#[test]
fn a_claimed_task_whose_pull_requests_all_merged_is_done_with_its_reason() {
    let store = seeded(vec![claimed(&[7, 8])]);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Merged);
    finish(&store, reading(&host))
        .execute_noted(&slug("a"), "Shipped by hand in #7 and #8", None)
        .unwrap();
    let fields = current(&store);
    assert_eq!(
        (
            fields.status,
            fields.resolution,
            fields.reason.as_deref(),
            fields.claim
        ),
        (
            TaskStatus::Done,
            Some(Resolution::DoneWithoutRun),
            Some("Shipped by hand in #7 and #8"),
            None
        )
    );
    assert_eq!(host.asked(), [7, 8], "each read on the host");
}

#[test]
fn an_open_pull_request_is_named_and_nothing_written() {
    let store = seeded(vec![claimed(&[7, 8])]);
    let before = current(&store);
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Merged);
    host.set(8, PullRequestState::Open);
    let refused = finish(&store, reading(&host)).execute_noted(&slug("a"), "By hand", None);
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::PrsOpen(vec![8])))
    );
    assert_eq!(current(&store), before);
}

#[test]
fn an_unreadable_host_is_unavailable_and_none_is_no_host() {
    let store = seeded(vec![claimed(&[7])]);
    let host = MemoryPullRequests::default();
    host.fail("the host hung up");
    let refused = finish(&store, reading(&host)).execute_noted(&slug("a"), "By hand", None);
    assert_eq!(
        refused,
        Err(ProjectBoardError::PullRequests(
            PullRequestStatesError::Unavailable("the host hung up".into())
        ))
    );
    let none = Err(NoPullRequestHost::NotAPullRequestHost);
    let refused = finish(&store, none).execute_noted(&slug("a"), "By hand", None);
    assert_eq!(
        refused,
        Err(ProjectBoardError::NoPullRequestHost(
            NoPullRequestHost::NotAPullRequestHost
        ))
    );
    assert_eq!(current(&store).status, TaskStatus::Claimed);
}

/// The rule judges the board's tasks before any host is read: a task a
/// dependency holds back is refused as such.
#[test]
fn a_task_held_back_by_a_dependency_is_refused_before_the_host_is_read() {
    let dependency = TaskFields {
        status: TaskStatus::Draft,
        ..fields("dep")
    };
    let held = TaskFields {
        depends_on: vec![slug("dep")],
        ..claimed(&[7])
    };
    let store = seeded(vec![dependency, held]);
    let host = MemoryPullRequests::default();
    host.fail("must not be asked");
    let refused = finish(&store, reading(&host)).execute_noted(&slug("a"), "By hand", None);
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::DependencyNotDone {
            undone: vec![slug("dep")],
            missing: vec![],
        }))
    );
    assert_eq!(host.asked(), Vec::<u64>::new());
}

#[test]
fn done_ticks_without_run_settles_outcomes_and_keeps_its_task_reason() {
    use crate::domain::project_board::entities::task_parts::DescriptionForm;
    let description =
        "## Goal\nAnswer the question.\n\n## Done when\n- [ ] Answer given\n- [x] Context retained";
    let store = seeded(vec![TaskFields {
        description: description.into(),
        description_form: DescriptionForm::Sectioned,
        ..fields("a")
    }]);
    let done = finish(&store, Err(NoPullRequestHost::NotAPullRequestHost))
        .execute_noted(&slug("a"), "Answered in chat", None)
        .unwrap();
    assert_eq!(done.fields().status, TaskStatus::Done);
    assert_eq!(done.fields().reason.as_deref(), Some("Answered in chat"));
    assert_eq!(done.fields(), &current(&store));
    assert_eq!(
        done.fields().description,
        description.replace("- [ ]", "- [x]")
    );
}

#[test]
fn done_ticks_refused_completion_preserves_unchecked_outcomes_without_writes() {
    use crate::domain::project_board::entities::task_parts::DescriptionForm;
    let store = seeded(vec![TaskFields {
        description: "## Goal\nShip.\n\n## Done when\n- [ ] Merged".into(),
        description_form: DescriptionForm::Sectioned,
        ..claimed(&[7])
    }]);
    let before = current(&store);
    let writes = store.writes();
    let host = MemoryPullRequests::default();
    host.set(7, PullRequestState::Open);
    assert_eq!(
        finish(&store, reading(&host)).execute_noted(&slug("a"), "By hand", None),
        Err(ProjectBoardError::Rule(BoardRuleError::PrsOpen(vec![7])))
    );
    assert_eq!(current(&store), before);
    assert_eq!(store.writes(), writes);
}
