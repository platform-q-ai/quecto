//! Contract for [`SubagentLifecycleRepository`] (#1934): the repository
//! stores only domain-validated lifecycle states and serves the lineage the
//! use cases route over. Prepare freezes through it, release thaws through
//! it, execute terminates through it.
use std::sync::Arc;

use quecto::application::subagents::dto::ReleaseOutcome;
use quecto::application::subagents::ports::SubagentLifecycleRepository;
use quecto::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::teardown_fixture::{Harness, Lifecycle, root_tree};

#[test]
fn repository_is_object_safe_and_serves_the_lineage_it_was_given() {
    let repository: Arc<dyn SubagentLifecycleRepository> = Lifecycle::new(root_tree());
    assert_eq!(repository.lifecycle(), HarnessLifecycleState::Accepting);
    assert_eq!(repository.lineage(), root_tree());
    let direct: Vec<_> = repository
        .lineage()
        .direct_children()
        .map(|child| child.uuid.as_str().to_owned())
        .collect();
    assert_eq!(direct, ["A", "D"]);
}

#[tokio::test]
async fn transaction_drives_freeze_thaw_and_terminate_through_the_repository() {
    let harness = Harness::new(root_tree());
    let first = harness.prepared(ShutdownReason::ParentShutdown);
    assert_eq!(harness.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    assert_eq!(
        harness.prepare.release(&first.token),
        Ok(ReleaseOutcome::Released)
    );
    assert_eq!(
        harness.lifecycle.lifecycle(),
        HarnessLifecycleState::Accepting
    );
    let second = harness.prepared(ShutdownReason::ParentShutdown);
    harness.execute.execute(&second.token).await.unwrap();
    assert_eq!(
        *harness.lifecycle.transitions.lock().unwrap(),
        [
            HarnessLifecycleState::Frozen,
            HarnessLifecycleState::Accepting,
            HarnessLifecycleState::Frozen,
            HarnessLifecycleState::Terminated,
        ]
    );
}
