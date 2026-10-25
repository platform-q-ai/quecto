//! Epic #2705 B8: a board server's end judges the run on what its client
//! saw (its coordinator still running, its swarm's kept board) and on the
//! remote's revisions as read before the write, never asking git or a
//! front end inside the transaction.
use super::*;
use crate::application::project_board::run_facts::{
    KeptViews, RunFacts, SeenLiveness, StatedRemoteRevisions,
};
use crate::application::project_board::use_cases::fixtures_tests::Remote;

const DEF: &str = "def456def456def456def456def456def456def4";

/// Ada's end at 10:30 on a board server: `facts` as its client sent them,
/// the remote's revisions as `remote` answered before the write.
fn served_ender(store: &MemoryBoardStore, facts: &RunFacts, remote: &Remote) -> EndRun {
    let revisions = StatedRemoteRevisions::read(remote, &facts.named_commits());
    EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        Arc::new(revisions),
        facts.liveness((&slug("board-store"), 1)),
        facts.reports((&slug("board-store"), 1)),
    )
    .unwrap()
}

fn facts(liveness: SeenLiveness, ended: Option<SwarmRunView>) -> RunFacts {
    RunFacts {
        liveness,
        reports: Some(KeptViews { ended, left: None }),
    }
}

/// Every item done, `store`'s evidence at [`DEF`].
fn done_at_def() -> SwarmRunView {
    let mut done = swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3]);
    done.items[1].evidence[0].revision = DEF.into();
    done
}

#[test]
fn an_end_whose_client_saw_its_coordinator_running_is_refused_run_live() {
    let (store, _) = stranded();
    let writes = store.writes();
    let seen = facts(SeenLiveness::Seen(Some("7f3a".into())), None);
    let refused =
        served_ender(&store, &seen, &Remote::holding_all()).execute(&ending(RunEndOutcome::Failed));
    assert_eq!(
        refused,
        Err(ProjectBoardError::RunLive {
            task: slug("board-store"),
            run: 1,
            agent: "7f3a".into(),
        })
    );
    assert_eq!(store.writes(), writes, "nothing written");
    let stopped = facts(SeenLiveness::Seen(None), None);
    let ended = served_ender(&store, &stopped, &Remote::holding_all())
        .execute(&ending(RunEndOutcome::Failed))
        .unwrap();
    assert_eq!(ended.liveness, LivenessCheck::Checked, "the client looked");
}

#[test]
fn a_success_whose_evidence_the_remote_lacks_is_refused_not_pushed() {
    let (store, _) = stranded();
    let writes = store.writes();
    let kept = facts(SeenLiveness::Seen(None), Some(done_at_def()));
    let remote = Remote::missing(&[DEF]);
    let refused = served_ender(&store, &kept, &remote).execute(&ending(RunEndOutcome::Succeeded));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::NotPushed {
            run: 1,
            revisions: 1
        }))
    );
    assert_eq!(remote.asks().len(), 1, "read once, before the write");
    assert_eq!(store.writes(), writes, "nothing written");
    let pushed = served_ender(&store, &kept, &Remote::holding_all())
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap();
    assert_eq!(pushed.run.fields().outcome, Some(RunOutcome::Succeeded));
    assert_eq!(pushed.report, RunReport::Kept);
}

/// A remote not read before the write (past its budget) refuses a
/// success, as a remote that cannot be read does.
#[test]
fn a_success_is_refused_while_the_remote_went_unread() {
    let (store, _) = stranded();
    let kept = facts(SeenLiveness::Seen(None), Some(done_at_def()));
    let ender = EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        Arc::new(StatedRemoteRevisions::unread(
            "not read within its time budget",
        )),
        kept.liveness((&slug("board-store"), 1)),
        kept.reports((&slug("board-store"), 1)),
    )
    .unwrap();
    let refused = ender.execute(&ending(RunEndOutcome::Succeeded));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Store(BoardStoreError::Unavailable(
            "not read within its time budget".into()
        )))
    );
}
