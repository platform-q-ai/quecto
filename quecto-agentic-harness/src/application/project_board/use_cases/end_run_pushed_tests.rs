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

/// A commit by a 12-digit prefix, and one by its full id.
const ABC: &str = "abc123abc123";
const DEF: &str = "def456def456def456def456def456def456def4";

/// A swarm that did every item: `api` and `docs` at commit [`ABC`],
/// `store` at [`DEF`] and at a bundle (no commit); and an extra item no
/// plan item takes, at a commit of its own.
fn done_at_two_revisions() -> SwarmRunView {
    let mut done = swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3]);
    done.items[0].evidence[0].revision = ABC.into();
    done.items[1].evidence[0].revision = DEF.into();
    done.items[1].evidence.push(
        crate::application::project_board::fold_back::SwarmEvidence {
            artifact: "red-tests.bundle".into(),
            revision: "red-tests.bundle".into(),
        },
    );
    done.items[2].evidence[0].revision = ABC.into();
    done.items.push(swarm_item(
        "extra: Not in the plan",
        SwarmItemState::Completed,
        None,
        &[("notes.md", "0ff0ff0ff0ff")],
    ));
    done
}

#[test]
fn a_success_over_evidence_never_pushed_is_refused_and_nothing_written() {
    let (store, _) = stranded();
    let writes = store.writes();
    let remote = Remote::missing(&[DEF]);
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
        [vec![ABC.to_string(), DEF.to_string()]],
        "once, each commit the recorded items' evidence names: never a bundle, nor an \
         extra item's"
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

/// Round 1 (#2670): every other rule judges an end first, so a remote that
/// cannot be read hides no undone item, and an end refused anyway never
/// waits on the network.
#[test]
fn every_other_rule_is_judged_before_the_remote_is_asked() {
    let (store, _) = stranded();
    let mut short = done_at_two_revisions();
    short.items[1].state = SwarmItemState::Submitted;
    let remote = Remote::down();
    assert_eq!(
        ender_asking(&store, Some(short), Arc::clone(&remote))
            .execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Rule(BoardRuleError::ItemsNotDone {
            run: 1,
            items: vec![slug("store")],
            report: SwarmReport::Kept,
        }))
    );
    let bob = EndRun::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        Arc::clone(&remote) as Arc<dyn RemoteRevisions>,
        None,
        Some(Arc::new(Kept(std::sync::Mutex::new(Some(
            done_at_two_revisions(),
        ))))),
    )
    .unwrap();
    assert_eq!(
        bob.execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolder))
    );
    assert!(remote.asks().is_empty(), "never asked: {:?}", remote.asks());
}

/// Round 1 (#2670): evidence that names no commit (a digest, a bundle, a
/// branch) is not checked, so a run whose items name none succeeds
/// without asking the remote.
#[test]
fn a_success_whose_evidence_names_no_commit_is_recorded_unchecked() {
    let (store, _) = stranded();
    let remote = Remote::down();
    let unnamed = swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3]);
    let ended = ender_asking(&store, Some(unnamed), Arc::clone(&remote))
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap();
    assert_eq!(ended.run.fields().outcome, Some(RunOutcome::Succeeded));
    assert!(remote.asks().is_empty());
}

/// A commit is named by its full id or a prefix of 12 hex digits or more,
/// in either case; anything else names none.
#[test]
fn a_commit_is_named_by_twelve_hex_digits_or_more() {
    use crate::application::project_board::fold_back::names_commit;
    for named in [ABC, DEF, &DEF.to_uppercase(), &"e".repeat(64)] {
        assert!(names_commit(named), "{named}");
    }
    for unnamed in [
        "abc123abc12",
        &"e".repeat(65),
        "red-tests.bundle",
        "main",
        "sha256:abc123abc123",
        "",
        "abc123abc12g",
    ] {
        assert!(!names_commit(unnamed), "{unnamed}");
    }
}
