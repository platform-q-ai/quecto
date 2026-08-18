use super::super::tests::*;
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolPolicyRequest,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use std::sync::Arc;

#[test]
fn immediate_persist_failure_does_not_emit_success_event_or_retained_overlay() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_cb = events.clone();
    agent.set_progress_callback(Some(Arc::new(move |event| {
        events_cb.lock().unwrap().push(event);
    })));
    agent.set_tool_policy_persistence(Some(Arc::new(|_| Err("config write failed".to_string()))));
    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::None,
        "durable disable",
    )]);
    request.persist = true;

    let reconciliation = agent
        .request_tool_policy(request, ToolPolicyApplyMode::ImmediateIfIdle)
        .expect("idle request returns reconciliation even when persistence fails");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::PersistenceFailed
    );
    assert!(
        reconciliation.results[0]
            .reason
            .contains("config write failed"),
        "persist error should be reported on the immediate ack reconciliation"
    );
    assert!(
        events.lock().unwrap().iter().all(|event| !matches!(
            event,
            crate::domain::agent::AgentProgressEvent::ToolPolicyChanged { .. }
        )),
        "failed immediate persistence must not emit a success-looking tool_policy_changed event"
    );
    assert!(
        !agent
            .tool_policy_state
            .lock()
            .unwrap()
            .disabled_tools
            .contains("alpha"),
        "failed durable immediate request must not remain in same-process retained overlay"
    );
}

#[test]
fn immediate_persist_patch_request_reaches_registry_with_persist_flag() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::Parent,
        "durable parent preference",
    )]);
    request.persist = true;
    request.correlation_id = Some("persist-immediate".into());

    let reconciliation = agent
        .request_tool_policy(request, ToolPolicyApplyMode::ImmediateIfIdle)
        .expect("idle request applies immediately");

    assert_eq!(
        reconciliation.correlation_id.as_deref(),
        Some("persist-immediate")
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction,
        "mock registry returns this sentinel only when AgentLoop preserves persist:true"
    );
}

#[test]
fn queued_persist_patch_request_persists_after_live_registry_apply() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let persisted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let persisted_cb = persisted.clone();
    agent.set_tool_policy_persistence(Some(std::sync::Arc::new(move |reconciliation| {
        persisted_cb
            .lock()
            .unwrap()
            .push(reconciliation.results[0].status);
        Ok(())
    })));
    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::Parent,
        "durable parent preference",
    )]);
    request.persist = true;
    request.correlation_id = Some("persist-queued".into());

    assert!(
        agent
            .request_tool_policy(request, ToolPolicyApplyMode::AtNextTurnBoundary)
            .is_none()
    );
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued request drains");

    assert_eq!(
        reconciliation.correlation_id.as_deref(),
        Some("persist-queued")
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction,
        "queued persist must reach the live registry with persist:true so runtime-tool reconnects retain the configured ceiling"
    );
    assert_eq!(
        persisted.lock().unwrap().as_slice(),
        &[ToolPolicyMutationStatus::BlockedByRestriction],
        "queued persist writes the live persisted reconciliation via the persistence callback"
    );
}
