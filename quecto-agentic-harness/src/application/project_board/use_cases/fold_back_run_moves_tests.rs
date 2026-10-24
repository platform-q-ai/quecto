//! #2705 T5: a fold-back writes its task's status in the same change as
//! the run, as an end_run does.
use super::*;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

fn done_swarm(end: SwarmEnd) -> SwarmRunView {
    let item = |text: &str| swarm_item(text, SwarmItemState::Completed, None, &[]);
    ended(
        end,
        vec![item("api: Build the api"), item("store: Build the store")],
    )
}

#[test]
fn a_fold_back_moves_its_task_by_how_its_swarm_ended_in_one_revision() {
    for (end, want) in [
        (SwarmEnd::Succeeded, TaskStatus::Review),
        (SwarmEnd::Blocked, TaskStatus::Blocked),
        (SwarmEnd::Cancelled, TaskStatus::Ready),
        (SwarmEnd::Failed, TaskStatus::Ready),
        (SwarmEnd::BudgetExhausted, TaskStatus::Ready),
    ] {
        let (store, held, _) = started();
        let writes = store.writes();
        let folded = fold_back(&store)
            .execute(&slug("board-store"), 1, &done_swarm(end))
            .unwrap();
        assert!(matches!(folded, FoldedBack::Folded(_)), "{end:?}");
        assert_eq!(store.writes(), writes + 1, "{end:?}: one revision");
        let board = store.read().unwrap();
        let fields = board.tasks[0].fields();
        assert_eq!(fields.status, want, "{end:?}");
        let claim = match want {
            TaskStatus::Ready => None,
            _ => held.fields().claim.clone(),
        };
        assert_eq!(fields.claim, claim, "{end:?}");
        assert_eq!(Some(&fields.updated), board.runs[0].fields().ended.as_ref());
    }
}

/// A run another writer ended first is not folded, and its task is left
/// where that end moved it.
#[test]
fn a_run_ended_already_moves_nothing_more() {
    let (store, _, _) = started();
    fold_back(&store)
        .execute(&slug("board-store"), 1, &done_swarm(SwarmEnd::Failed))
        .unwrap();
    let writes = store.writes();
    let again = fold_back(&store)
        .execute(&slug("board-store"), 1, &done_swarm(SwarmEnd::Succeeded))
        .unwrap();
    assert_eq!(again, FoldedBack::Ended(1));
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!(
        store.read().unwrap().tasks[0].fields().status,
        TaskStatus::Ready
    );
}
