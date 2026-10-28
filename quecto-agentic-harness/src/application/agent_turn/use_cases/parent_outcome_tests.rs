//! #2708: the parent turn-outcome runtime, driven through its public use
//! case and the bound declaration port the Native tool holds.
use super::*;
use crate::application::agent_turn::ports::{BoundTurnDeclaration, DeclareTurnOutcome};
use crate::domain::agents::value_objects::turn_outcome::{
    CORRECTION_NOTE, LIVE_REASON, STALLED_REASON, TurnAdmissionRecord, TurnOutcome, WorkLiveness,
};
use crate::domain::turn_declaration::{LaunchAuthority, TurnIntent};

fn mint() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("epoch-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn runtime() -> ParentOutcomeRuntime {
    ParentOutcomeRuntime::new(mint)
}

fn external(runtime: &ParentOutcomeRuntime, episode: &str, turn: &str) -> ActiveTurn {
    runtime
        .admit(
            "cli:root",
            TurnAdmission::External {
                episode_id: episode.into(),
            },
            turn,
        )
        .expect("an external instruction is always admitted")
}

fn declare(runtime: &ParentOutcomeRuntime, intent: TurnIntent, reason: &str) {
    let identity = runtime.capture().expect("a turn is active");
    runtime
        .declare(LaunchAuthority::TopLevelParent, &identity, intent, reason)
        .expect("valid declaration");
}

fn published(verdict: ParentVerdict) -> TurnOutcomeRecord {
    match verdict {
        ParentVerdict::Published(record) => record,
        other => panic!("expected a published outcome, got {other:?}"),
    }
}

#[test]
fn issue_2708_parent_runtime_admission_freezes_the_committed_identity() {
    let runtime = runtime();
    let epoch = runtime.view().epoch;
    let turn = external(&runtime, "episode-1", "opener-1");
    assert_eq!(
        turn.admission(),
        TurnAdmissionRecord {
            session_key: "cli:root".into(),
            outcome_epoch: epoch.clone(),
            episode_id: "episode-1".into(),
            turn_id: "opener-1".into(),
        }
    );
    let captured = runtime.capture().unwrap();
    assert_eq!(captured.turn_id, "opener-1");
    assert_eq!(captured.outcome_epoch, epoch);
}

#[test]
fn issue_2708_parent_runtime_empty_turn_is_corrected_once_then_stalls() {
    let runtime = runtime();
    let first = external(&runtime, "episode-1", "opener-1");
    assert_eq!(
        runtime.complete(first, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct
    );
    // The correction is the same episode under a new turn.
    let correction = runtime
        .admit("cli:root", TurnAdmission::Correction, "correction-1")
        .expect("the one correction is admitted");
    assert_eq!(correction.admission().episode_id, "episode-1");
    assert_eq!(correction.admission().turn_id, "correction-1");
    let stalled = published(runtime.complete(correction, TurnEnd::Completed(WorkLiveness::Empty)));
    assert_eq!(stalled.outcome, TurnOutcome::Stalled);
    assert_eq!(stalled.reason, STALLED_REASON);
    assert_eq!(stalled.episode_id, "episode-1");
    assert_eq!(stalled.turn_id, "correction-1");
    assert_eq!(stalled.sequence, 1);
    // No third request: no second correction, no automatic turn.
    assert!(
        runtime
            .admit("cli:root", TurnAdmission::Correction, "correction-2")
            .is_none()
    );
    assert!(!runtime.admits_automatic());
    assert_eq!(runtime.view().latest, Some(stalled));
    assert!(!CORRECTION_NOTE.is_empty());
}

#[test]
fn issue_2708_parent_runtime_a_correction_needs_a_correct_verdict() {
    let runtime = runtime();
    assert!(
        runtime
            .admit("cli:root", TurnAdmission::Correction, "c")
            .is_none(),
        "no episode, no correction"
    );
    let _turn = external(&runtime, "episode-1", "opener-1");
    assert!(
        runtime
            .admit("cli:root", TurnAdmission::Correction, "c")
            .is_none(),
        "a correction is admitted only after the policy asked for one"
    );
}

#[test]
fn issue_2708_parent_runtime_live_work_waits_and_a_work_wake_renews_the_correction() {
    let runtime = runtime();
    let turn = external(&runtime, "episode-1", "opener-1");
    let waiting = published(runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Live)));
    assert_eq!(waiting.outcome, TurnOutcome::Waiting);
    assert_eq!(waiting.reason, LIVE_REASON);
    assert!(runtime.admits_automatic(), "waiting is not terminal");
    let wake = runtime
        .admit("cli:root", TurnAdmission::WorkWake, "note-1")
        .expect("a work wake continues the episode");
    assert_eq!(wake.admission().episode_id, "episode-1");
    assert_eq!(
        runtime.complete(wake, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct,
        "admitted live work grants the next correction"
    );
}

#[test]
fn issue_2708_parent_runtime_nudges_and_reads_never_replenish_the_correction() {
    let runtime = runtime();
    let turn = external(&runtime, "episode-1", "opener-1");
    assert_eq!(
        runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct
    );
    let correction = runtime
        .admit("cli:root", TurnAdmission::Correction, "c-1")
        .unwrap();
    let waiting = published(runtime.complete(correction, TurnEnd::Completed(WorkLiveness::Live)));
    assert_eq!(waiting.outcome, TurnOutcome::Waiting);
    let _ = runtime.view();
    let _ = runtime.admits_automatic();
    let nudge = runtime
        .admit("cli:root", TurnAdmission::Nudge, "nudge-1")
        .expect("a nudge continues a waiting episode");
    let stalled = published(runtime.complete(nudge, TurnEnd::Completed(WorkLiveness::Empty)));
    assert_eq!(stalled.outcome, TurnOutcome::Stalled);
    assert_eq!(stalled.sequence, waiting.sequence + 1);
}

#[test]
fn issue_2708_parent_runtime_declared_terminals_fence_automatic_turns_until_external() {
    let cases = [
        (TurnIntent::Done, TurnOutcome::Done),
        (TurnIntent::Blocked, TurnOutcome::NeedsUser),
        (TurnIntent::Asking, TurnOutcome::NeedsUser),
    ];
    let liveness = [
        WorkLiveness::Live,
        WorkLiveness::Empty,
        WorkLiveness::Unknown,
    ];
    for ((intent, outcome), liveness) in cases
        .into_iter()
        .flat_map(|case| liveness.into_iter().map(move |live| (case, live)))
    {
        let runtime = runtime();
        let turn = external(&runtime, "episode-1", "opener-1");
        declare(&runtime, intent, "  Which repository?  ");
        let record = published(runtime.complete(turn, TurnEnd::Completed(liveness)));
        assert_eq!(record.outcome, outcome);
        assert_eq!(record.reason, "Which repository?");
        assert!(!runtime.admits_automatic(), "{intent:?} fences");
        for automatic in [TurnAdmission::WorkWake, TurnAdmission::Nudge] {
            assert!(runtime.admit("cli:root", automatic, "auto").is_none());
        }
        // A fresh external instruction reopens a new episode.
        let fresh = external(&runtime, "episode-2", "opener-2");
        assert!(runtime.admits_automatic());
        declare(&runtime, TurnIntent::Done, "Finished");
        let done = published(runtime.complete(fresh, TurnEnd::Completed(WorkLiveness::Empty)));
        assert_eq!(
            (done.outcome, done.episode_id.as_str(), done.sequence),
            (TurnOutcome::Done, "episode-2", 2)
        );
    }
}

#[test]
fn issue_2708_parent_runtime_interrupted_turns_settle_nothing_and_drop_their_declaration() {
    let runtime = runtime();
    let turn = external(&runtime, "episode-1", "opener-1");
    declare(&runtime, TurnIntent::Done, "Finished");
    assert_eq!(
        runtime.complete(turn, TurnEnd::Interrupted),
        ParentVerdict::Unsettled
    );
    assert!(runtime.capture().is_none(), "no turn is active");
    assert_eq!(runtime.view().latest, None);
    let next = external(&runtime, "episode-2", "opener-2");
    assert_eq!(
        runtime.complete(next, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct,
        "the interrupted turn's declaration is not inherited"
    );
}

#[test]
fn issue_2708_parent_runtime_a_superseded_turn_cannot_settle() {
    let runtime = runtime();
    let old = external(&runtime, "episode-1", "opener-1");
    let old_identity = runtime.capture().unwrap();
    let new = external(&runtime, "episode-2", "opener-2");
    assert_eq!(
        runtime.complete(old, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Unsettled
    );
    assert!(
        runtime
            .declare(
                LaunchAuthority::TopLevelParent,
                &old_identity,
                TurnIntent::Done,
                "late"
            )
            .is_err(),
        "a stale identity cannot declare"
    );
    declare(&runtime, TurnIntent::Done, "Finished");
    assert_eq!(
        published(runtime.complete(new, TurnEnd::Completed(WorkLiveness::Empty))).turn_id,
        "opener-2"
    );
}

#[test]
fn issue_2708_parent_runtime_history_replacement_mints_a_new_epoch() {
    let runtime = runtime();
    let turn = external(&runtime, "episode-1", "opener-1");
    declare(&runtime, TurnIntent::Done, "Finished");
    let done = published(runtime.complete(turn, TurnEnd::Completed(WorkLiveness::Empty)));
    let before = runtime.view();
    assert_eq!(before.latest, Some(done.clone()));
    let pending = external(&runtime, "episode-2", "opener-2");
    runtime.history_replaced();
    let after = runtime.view();
    assert_ne!(after.epoch, before.epoch);
    assert_eq!(after.latest, None);
    assert!(runtime.admits_automatic());
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.complete(pending, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Unsettled,
        "a turn of the replaced history settles nothing"
    );
    assert!(
        runtime
            .admit("cli:root", TurnAdmission::WorkWake, "w")
            .is_none(),
        "the new epoch has no episode yet"
    );
    let fresh = external(&runtime, "episode-3", "opener-3");
    assert_eq!(fresh.admission().outcome_epoch, after.epoch);
    declare(&runtime, TurnIntent::Done, "Again");
    let again = published(runtime.complete(fresh, TurnEnd::Completed(WorkLiveness::Empty)));
    assert!(again.sequence > done.sequence, "sequence never goes back");
}

#[test]
fn run21_open_workflow_nudge_preserves_spent_budget_new_episode_resets() {
    let runtime = runtime();
    let first = external(&runtime, "open", "first");
    assert!(matches!(
        runtime.complete(first, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct
    ));
    let correction = runtime
        .admit("cli:root", TurnAdmission::Correction, "correction")
        .unwrap();
    let waiting = published(runtime.complete(correction, TurnEnd::Completed(WorkLiveness::Live)));
    assert_eq!(waiting.outcome, TurnOutcome::Waiting);
    let nudge = runtime
        .admit("cli:root", TurnAdmission::FreshWake, "workflow-information")
        .unwrap();
    assert_eq!(
        nudge.admission().episode_id,
        "open",
        "still-open workflow nudge is same episode"
    );
    let settled = runtime.complete(nudge, TurnEnd::Completed(WorkLiveness::Empty));
    assert!(
        matches!(settled, ParentVerdict::Published(ref record) if record.outcome == TurnOutcome::Stalled),
        "nudge cannot replenish spent correction: {settled:?}"
    );
    let next = external(&runtime, "genuine-next", "next");
    assert!(
        matches!(
            runtime.complete(next, TurnEnd::Completed(WorkLiveness::Empty)),
            ParentVerdict::Correct
        ),
        "genuine new episode gets one correction"
    );
}
