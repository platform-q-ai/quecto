//! Contract for [`OwnerExitAnnouncement`] (#2070): held by the client that
//! announced it, withdrawn when that client goes; an executed shutdown reads
//! it once, before the fleet is asked, and tears the fleet down on the
//! owner's authority when it is raised and on the harness's when it is not.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use quecto::application::subagents::ports::{OwnerExitAnnouncement, TerminationCause};
use quecto::domain::subagent_teardown::ShutdownReason;

use super::teardown_fixture::{Harness, OwnerExit, root_tree};

#[test]
fn port_is_object_safe_held_by_its_announcer_and_withdrawn_only_by_that_client() {
    let port: Arc<dyn OwnerExitAnnouncement> = Arc::new(OwnerExit::default());
    assert!(!port.announced());
    port.announce(1);
    assert!(port.announced());
    port.withdraw(2);
    assert!(port.announced(), "another client's close leaves it raised");
    port.withdraw(1);
    assert!(!port.announced());
}

#[tokio::test]
async fn an_announced_exit_tears_the_fleet_down_on_the_owners_authority() {
    let harness = Harness::new(root_tree());
    harness.owner_exit.announce(1);
    let prepared = harness.prepared(ShutdownReason::TerminationSignal);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(outcome.owner_exit);
    let causes: Vec<TerminationCause> = harness
        .rows
        .compensated
        .lock()
        .unwrap()
        .iter()
        .map(|(_, cause)| *cause)
        .collect();
    assert!(!causes.is_empty());
    assert!(
        causes
            .iter()
            .all(|cause| *cause == TerminationCause::OwnerTeardown),
        "{causes:?}"
    );
}

#[tokio::test]
async fn an_unannounced_shutdown_is_the_harnesss_word_and_an_announcement_mid_run_changes_nothing()
{
    let harness = Harness::new(root_tree());
    let prepared = harness.prepared(ShutdownReason::TerminationSignal);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(!outcome.owner_exit);
    // Announced only after the fleet step decided: a re-drive of the same
    // admission keeps the harness's authority.
    harness.owner_exit.announce(1);
    let again = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(!again.owner_exit);
    assert_eq!(harness.retained.asked.load(Ordering::SeqCst), 0);
    let causes = harness.rows.compensated.lock().unwrap().clone();
    assert!(
        causes
            .iter()
            .all(|(_, cause)| *cause == TerminationCause::FleetTeardown),
        "{causes:?}"
    );
}
