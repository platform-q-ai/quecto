//! Contract for [`ShutdownClock`] (#1934): the clock is injected, never
//! read from the system inside the application, and it only feeds token
//! minting — it never decides whether a shutdown is admitted.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use quecto::application::subagents::dto::ReleaseOutcome;
use quecto::application::subagents::ports::{ShutdownClock, ShutdownInstant};
use quecto::domain::subagent_teardown::ShutdownReason;

use super::teardown_fixture::{Clock, Harness, root_tree};

#[test]
fn port_is_object_safe_and_instants_are_ordered_milliseconds() {
    let clock = Arc::new(Clock(AtomicU64::new(5)));
    let port: Arc<dyn ShutdownClock> = clock.clone();
    assert_eq!(port.now(), ShutdownInstant(5));
    clock.0.store(6, Ordering::SeqCst);
    assert!(port.now() > ShutdownInstant(5));
}

#[test]
fn tokens_differ_across_admissions_even_on_a_frozen_clock() {
    let harness = Harness::new(root_tree());
    let first = harness.prepared(ShutdownReason::ParentShutdown);
    assert_eq!(
        harness.prepare.release(&first.token),
        Ok(ReleaseOutcome::Released)
    );
    let second = harness.prepared(ShutdownReason::ParentShutdown);
    assert_ne!(first.token, second.token);
    // Two harnesses on the same clock reading never share a token either.
    let other = Harness::new(root_tree());
    let third = other.prepared(ShutdownReason::ParentShutdown);
    assert_ne!(first.token, third.token);
    assert_ne!(second.token, third.token);
}
