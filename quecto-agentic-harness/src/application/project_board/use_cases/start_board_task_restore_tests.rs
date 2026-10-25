//! #2705 T5: a failed launch's undo is its run's fold-back as failed, one
//! write after the start's, and it leaves the task where the start found
//! it: a task the start claimed goes back to ready, released; one held in
//! review or blocked goes back there, its claim kept.
use super::{Launcher, confirmed, plan, seeded, starter, task_and_runs};
use crate::application::project_board::dto::{ProjectBoardError, RunAfterUndo};
use crate::application::project_board::use_cases::fixtures_tests::{at, person};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};

/// Task `a` in status `from`, held by Ada until 11:00 when it holds a
/// claim.
fn held_from(from: TaskStatus) -> impl FnOnce(TaskFields) -> TaskFields {
    move |fields: TaskFields| TaskFields {
        status: from,
        claim: (from != TaskStatus::Ready).then(|| Claim {
            holder: person("Ada"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        ..fields
    }
}

/// A task the start claimed (ready, or claimed by its starter) goes back
/// to ready, released: no task is left in progress without a run, and no
/// move follows.
#[test]
fn a_failed_launch_leaves_a_claimed_start_ready_and_released_in_one_write() {
    for from in [TaskStatus::Ready, TaskStatus::Claimed] {
        let store = seeded(held_from(from), vec![plan(1, PlanState::Accepted)], vec![]);
        let writes = store.writes();
        let refused = starter(&store, Some(Launcher::failing())).execute(&confirmed(1));
        assert!(
            matches!(
                &refused,
                Err(ProjectBoardError::LaunchFailed {
                    run_state: RunAfterUndo::Folded,
                    released: Some(true),
                    restored: None,
                    still_running: None,
                    ..
                })
            ),
            "{from:?}: {refused:?}"
        );
        assert_eq!(
            store.writes(),
            writes + 2,
            "{from:?}: the start and its undo"
        );
        let (task, runs) = task_and_runs(&store);
        let fields = task.fields();
        assert_eq!(
            (fields.status, fields.claim.clone()),
            (TaskStatus::Ready, None)
        );
        assert_eq!(runs[0].fields().outcome, Some(RunOutcome::Failed));
        let said = refused.unwrap_err().to_string();
        assert!(
            said.ends_with("run 1 was folded back as failed, and the task is ready again"),
            "{said}"
        );
    }
}

/// Review round 2 of S4c, kept by #2705 T5 review round 1: a task held in
/// review or blocked whose start failed to launch goes back there in the
/// undo's own write, its claim kept, so a review is still completed when
/// its pull requests merge.
#[test]
fn a_held_start_undone_goes_back_to_its_status() {
    for from in [TaskStatus::Review, TaskStatus::Blocked] {
        let store = seeded(held_from(from), vec![plan(1, PlanState::Accepted)], vec![]);
        let writes = store.writes();
        let refused = starter(&store, Some(Launcher::failing())).execute(&confirmed(1));
        assert_eq!(
            refused.as_ref().map(|_| ()),
            Err(&ProjectBoardError::LaunchFailed {
                reason: "no container runtime".into(),
                run: 1,
                run_state: RunAfterUndo::Folded,
                released: Some(false),
                restored: Some(from),
                still_running: None,
            }),
            "{from:?}"
        );
        assert_eq!(
            store.writes(),
            writes + 2,
            "{from:?}: the start and its undo"
        );
        let (task, runs) = task_and_runs(&store);
        assert_eq!(task.fields().status, from);
        let claim = task.fields().claim.clone().expect("still held");
        assert_eq!(claim.holder, person("Ada"));
        assert_eq!(runs[0].fields().outcome, Some(RunOutcome::Failed));
        assert_eq!(
            refused.unwrap_err().to_string(),
            format!(
                "the coordinator did not launch: no container runtime; run 1 was folded back as \
                 failed, and the task is back in {}, still yours",
                from.as_str()
            )
        );
    }
}
