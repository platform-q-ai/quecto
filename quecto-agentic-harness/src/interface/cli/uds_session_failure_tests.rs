use super::*;

#[cfg(test)]
#[test]
fn identical_failure_notifications_do_not_reopen_an_idle_turn() {
    let mut session = AgentSession::new("test".into(), "test".into());
    assert!(
        session
            .enqueue_subagent_notification("worker".into(), 1, "quota exhausted".into(), false)
            .is_retained()
    );
    assert_eq!(session.drain_pending().len(), 1);
    assert_eq!(
        session.enqueue_subagent_notification("worker".into(), 2, "quota exhausted".into(), false),
        NotificationEnqueueOutcome::Duplicate
    );
    assert!(session.drain_pending().is_empty());
    assert!(
        session
            .enqueue_subagent_notification("worker".into(), 3, "different failure".into(), false)
            .is_retained()
    );
    session.drain_pending();
    assert!(
        session
            .enqueue_subagent_notification("worker".into(), 4, "recovered".into(), true)
            .is_retained()
    );
    session.drain_pending();
    assert!(
        session
            .enqueue_subagent_notification("worker".into(), 5, "different failure".into(), false)
            .is_retained()
    );
}
