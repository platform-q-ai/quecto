//! #2666: a board run ended succeeded with its evidence at a commit its
//! coordinator made in the swarm's container and never pushed; the
//! teardown took the work with the box, and the board showed a success
//! over work that no longer existed. A success is recorded only when every
//! revision the recorded items' evidence names is on the board's remote.
use super::*;
use crate::application::project_board::ports::RemoteRevisions;

/// Ada's end of the run at 10:30, seeing no swarm running, reading the
/// board `kept` holds and asking `remote` which revisions it holds.
fn ender_asking(
    store: &MemoryBoardStore,
    kept: Option<SwarmRunView>,
    remote: Arc<Remote>,
) -> EndRun {
    EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        remote as Arc<dyn RemoteRevisions>,
        None,
        Some(Arc::new(Kept(std::sync::Mutex::new(kept)))),
    )
    .unwrap()
}

/// A swarm that did every item, item `store`'s evidence at `def456`, and
/// an extra item no plan item takes, at `0ff0ff`.
fn done_at_two_revisions() -> SwarmRunView {
    let mut done = swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3]);
    done.items[1].evidence[0].revision = "def456".into();
    done.items.push(swarm_item(
        "extra: Not in the plan",
        SwarmItemState::Completed,
        None,
        &[("notes.md", "0ff0ff")],
    ));
    done
}

#[test]
fn a_success_over_evidence_never_pushed_is_refused_and_nothing_written() {
    let (store, _) = stranded();
    let writes = store.writes();
    let remote = Remote::missing(&["def456"]);
    let refused = ender_asking(&store, Some(done_at_two_revisions()), Arc::clone(&remote))
        .execute(&ending(RunEndOutcome::Succeeded));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::NotPushed {
            run: 1,
            revisions: 1
        }))
    );
    assert_eq!(
        remote.asks(),
        [vec!["abc123".to_string(), "def456".to_string()]],
        "once, each revision the recorded items' evidence names, never an extra item's"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!(run_1(&store).fields().outcome, None, "the run stays open");
}

#[test]
fn a_success_over_evidence_on_the_remote_is_recorded() {
    let (store, _) = stranded();
    let remote = Remote::holding_all();
    let ended = ender_asking(&store, Some(done_at_two_revisions()), Arc::clone(&remote))
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap();
    assert_eq!(ended.run.fields().outcome, Some(RunOutcome::Succeeded));
    assert_eq!(ended.report, RunReport::Kept);
    assert_eq!(remote.asks().len(), 1);
}

#[test]
fn a_success_is_not_recorded_while_the_remote_cannot_be_read() {
    let (store, _) = stranded();
    let writes = store.writes();
    let refused = ender_asking(&store, Some(done_at_two_revisions()), Remote::down())
        .execute(&ending(RunEndOutcome::Succeeded));
    assert!(
        matches!(
            refused,
            Err(ProjectBoardError::Store(BoardStoreError::Unavailable(_)))
        ),
        "{refused:?}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
}

/// The remote is asked once however many passes the write takes (one
/// past the board's bytes is written again without its evidence).
#[test]
fn the_remote_is_asked_once_across_a_second_pass() {
    let (store, _) = stranded();
    store.fail_next_write(BoardStoreError::WriteTooLarge(
        "runs/ past its bytes".into(),
    ));
    let remote = Remote::holding_all();
    let ended = ender_asking(&store, Some(done_at_two_revisions()), Arc::clone(&remote))
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap();
    assert_eq!(ended.run.fields().outcome, Some(RunOutcome::Succeeded));
    assert_eq!(remote.asks().len(), 1, "asked once");
}

/// Only a success is judged by it: a stop, a failure or a loss never
/// asks, and records what the board showed as ever; nor does a success
/// with no board kept, which its undone items refuse first.
#[test]
fn only_a_success_with_evidence_asks_the_remote() {
    for outcome in [
        RunEndOutcome::Stopped,
        RunEndOutcome::Failed,
        RunEndOutcome::Lost,
    ] {
        let (store, _) = stranded();
        let remote = Remote::down();
        let ended = ender_asking(&store, Some(done_at_two_revisions()), Arc::clone(&remote))
            .execute(&ending(outcome));
        assert!(ended.is_ok(), "{outcome:?}: {ended:?}");
        assert!(remote.asks().is_empty(), "{outcome:?}");
    }
    let (store, _) = stranded();
    let remote = Remote::down();
    let refused =
        ender_asking(&store, None, Arc::clone(&remote)).execute(&ending(RunEndOutcome::Succeeded));
    assert!(
        matches!(
            refused,
            Err(ProjectBoardError::Rule(BoardRuleError::ItemsNotDone { .. }))
        ),
        "{refused:?}"
    );
    assert!(remote.asks().is_empty(), "no revision to ask about");
}
