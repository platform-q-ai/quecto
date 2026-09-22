//! The owner's exit (#2070): a shutdown the TUI announced beforehand is the
//! owner's word — its fleet teardown gives every swarm's box up and the
//! session's emptied `retained` environments are ended — while an
//! unannounced one keeps them all.
use std::sync::atomic::Ordering;

use super::tests::{Rig, protocol, rig};
use crate::application::subagents::ports::{OwnerExitAnnouncement, TerminationCause};
use crate::domain::subagent_teardown::ShutdownReason;

async fn shut_down(
    rig: &Rig,
    reason: ShutdownReason,
) -> crate::application::subagents::dto::ShutdownOutcome {
    let token = rig.prepare.execute(protocol(reason)).unwrap().token;
    rig.execute.execute(&token).await.unwrap()
}

#[tokio::test]
async fn an_announced_exit_tears_the_fleet_down_on_the_owners_authority_and_ends_retained_boxes() {
    let rig = rig();
    *rig.retained.answer.lock().unwrap() = vec![
        ("C3".to_string(), Ok(())),
        ("C7".to_string(), Err("kill refused".to_string())),
    ];
    rig.owner_exit.announce();
    // The TUI's ordinary exit reaches the harness as a bare signal.
    let outcome = shut_down(&rig, ShutdownReason::TerminationSignal).await;
    assert!(outcome.owner_exit);
    let causes = rig.fleet.compensation.calls();
    assert_eq!(causes.len(), 2);
    assert!(
        causes
            .iter()
            .all(|(_, cause)| *cause == TerminationCause::OwnerTeardown),
        "{causes:?}"
    );
    assert_eq!(rig.retained.asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        outcome.retained_environments,
        [
            ("C3".to_string(), Ok(())),
            ("C7".to_string(), Err("kill refused".to_string()))
        ]
    );
    assert!(outcome.exit_signalled);
}

#[tokio::test]
async fn an_unannounced_shutdown_is_not_the_owners_word_whatever_signalled_it() {
    for reason in [
        ShutdownReason::TerminationSignal,
        ShutdownReason::ParentConnectionLost,
        ShutdownReason::OperatorRequest,
    ] {
        let rig = rig();
        let outcome = shut_down(&rig, reason).await;
        assert!(!outcome.owner_exit, "{reason:?}");
        assert!(
            rig.fleet
                .compensation
                .calls()
                .iter()
                .all(|(_, cause)| *cause == TerminationCause::FleetTeardown),
            "{reason:?}"
        );
        assert_eq!(rig.retained.asked.load(Ordering::SeqCst), 0, "{reason:?}");
        assert!(outcome.retained_environments.is_empty(), "{reason:?}");
    }
}

#[tokio::test]
async fn the_retained_environments_are_ended_after_the_fleet_and_only_once() {
    // A re-driven admission must not ask twice: the step is recorded like
    // every other.
    let rig = rig();
    rig.owner_exit.announce();
    shut_down(&rig, ShutdownReason::TerminationSignal).await;
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::TerminationSignal))
        .unwrap()
        .token;
    let again = rig.execute.execute(&token).await.unwrap();
    assert!(again.owner_exit);
    assert_eq!(rig.retained.asked.load(Ordering::SeqCst), 1);
    assert_eq!(rig.fleet.compensation.calls().len(), 2);
}

#[tokio::test]
async fn the_authority_is_decided_once_an_announcement_after_the_fleet_ran_changes_nothing() {
    // The run is dropped while exit readiness is held — after the fleet
    // settled on the harness's authority. An announcement arriving now, and
    // the re-drive that follows, must not turn the shutdown into the
    // owner's: the fleet was already asked, and the retained boxes stay.
    let rig = rig();
    rig.exit.hold.store(true, Ordering::SeqCst);
    let token = rig
        .prepare
        .execute(protocol(ShutdownReason::TerminationSignal))
        .unwrap()
        .token;
    let joiner = tokio::spawn({
        let execute = rig.execute.clone();
        let token = token.clone();
        async move { execute.execute(&token).await }
    });
    while rig.exit.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    rig.spawner.abort_latest().await;
    assert!(joiner.await.unwrap().is_err());
    rig.owner_exit.announce();
    rig.exit.hold.store(false, Ordering::SeqCst);
    let outcome = rig.execute.execute(&token).await.unwrap();
    assert!(!outcome.owner_exit);
    assert!(outcome.retained_environments.is_empty());
    assert_eq!(rig.retained.asked.load(Ordering::SeqCst), 0);
}
