//! The optional policy collaborator is the same root guard through a port.
use quecto::application::agent_turn::dto::parent_outcome::{ParentVerdict, TurnAdmission, TurnEnd};
use quecto::application::agent_turn::ports::ParentOutcomeGuard;
use quecto::application::agent_turn::use_cases::parent_outcome::ParentOutcomeRuntime;
use quecto::domain::agents::value_objects::turn_outcome::{TurnOutcome, WorkLiveness};
use quecto::domain::turn_declaration::{LaunchAuthority, TurnIntent};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

fn mint() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed).to_string()
}

#[test]
fn effective_policy_revokes_old_authority_and_restores_only_at_outer_boundary() {
    let guard: Arc<dyn ParentOutcomeGuard> = Arc::new(ParentOutcomeRuntime::new(mint));
    let old = guard
        .admit(
            "cli:test",
            TurnAdmission::External {
                episode_id: "one".into(),
            },
            "old",
        )
        .unwrap();
    let identity = guard.capture().unwrap();
    guard.observe_policy(true); // A no-op preserves capture.
    assert_eq!(guard.capture().as_ref(), Some(&identity));
    guard.observe_policy(false);
    assert!(!guard.enabled());
    assert!(guard.capture().is_none());
    guard.observe_policy(true);
    assert!(!guard.enabled());
    assert!(
        guard
            .declare(
                LaunchAuthority::TopLevelParent,
                &identity,
                TurnIntent::Done,
                "stale"
            )
            .is_err()
    );
    assert_eq!(
        guard.complete(old, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Unsettled
    );
    guard.enter_outer_boundary();
    assert!(guard.enabled());
    let fresh = guard
        .admit("cli:test", TurnAdmission::FreshWake, "fresh")
        .unwrap();
    let fresh_identity = guard.capture().unwrap();
    assert_ne!(fresh_identity.outcome_epoch, identity.outcome_epoch);
    guard
        .declare(
            LaunchAuthority::TopLevelParent,
            &fresh_identity,
            TurnIntent::Done,
            "settled",
        )
        .unwrap();
    match guard.complete(fresh, TurnEnd::Completed(WorkLiveness::Empty)) {
        ParentVerdict::Published(record) => {
            assert_eq!(record.outcome, TurnOutcome::Done);
            assert_eq!(guard.view().latest, Some(record));
        }
        other => panic!("expected a completed settlement, got {other:?}"),
    }
    assert!(!guard.admits_automatic());
    guard.history_replaced();
    assert!(guard.view().latest.is_none());
    assert!(guard.admits_automatic());
}

#[test]
fn only_an_active_admitted_correction_has_the_empty_endturn_budget() {
    let guard: Arc<dyn ParentOutcomeGuard> = Arc::new(ParentOutcomeRuntime::new(mint));
    assert!(!guard.correcting());
    let turn = guard
        .admit(
            "cli:test",
            TurnAdmission::External {
                episode_id: "one".into(),
            },
            "prompt",
        )
        .unwrap();
    assert!(!guard.correcting());
    assert_eq!(
        guard.complete(turn, TurnEnd::Completed(WorkLiveness::Empty)),
        ParentVerdict::Correct
    );
    assert!(!guard.correcting()); // Owed is not admitted.
    let correction = guard
        .admit("cli:test", TurnAdmission::Correction, "correction")
        .unwrap();
    assert!(guard.correcting());
    assert!(
        matches!(guard.complete(correction, TurnEnd::Completed(WorkLiveness::Empty)), ParentVerdict::Published(record) if record.outcome == TurnOutcome::Stalled)
    );
    assert!(!guard.correcting());
}
