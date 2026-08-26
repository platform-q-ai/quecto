//! Coverage-focused tests for `agent_loop_policy` recovery paths: poisoned
//! tool-policy/pending-request mutexes must recover via `into_inner` and keep
//! the recorded state, and catalogue-change notifications must fire.

use super::super::tests::{MockProvider, MockRegistry, MockTool, test_config};
use super::super::*;
use super::mock_catalogue_entry;
use crate::domain::tool::{ToolPolicyMutation, ToolPolicyMutationStatus};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

fn make_policy_agent() -> AgentLoopImpl {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("alpha", "ok")));
    AgentLoopImpl::new(test_config(provider, Box::new(registry)))
}

/// Panic while holding the given mutex so it becomes poisoned.
fn poison<T>(mutex: &Mutex<T>) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _guard = mutex.lock().unwrap();
        panic!("poison for coverage test");
    }));
    assert!(result.is_err(), "closure must panic to poison the mutex");
    assert!(mutex.is_poisoned(), "mutex must be poisoned after panic");
}

#[test]
fn poisoned_tool_policy_state_recovers_and_keeps_recorded_scope() {
    let mut agent = make_policy_agent();
    {
        let mut policy = agent.tool_policy_state.lock().unwrap();
        policy.record_applied("alpha", ProfileAvailabilityScope::None);
    }
    poison(&agent.tool_policy_state);

    // tool_catalogue_entries recovers the poisoned lock (MockRegistry has no
    // catalogue entries, so it returns empty but must not panic).
    assert!(agent.tool_catalogue_entries().is_empty());

    // current_tool_definitions recovers the poisoned lock and still honours
    // the recorded scope: alpha stays hidden from the parent profile.
    assert!(
        agent.current_tool_definitions().is_empty(),
        "scope=None recorded before poisoning must still hide alpha"
    );

    // apply_persisted_tool_policy_entries recovers the poisoned lock and
    // clears runtime overlays, restoring alpha's visibility.
    let unknown = agent.apply_persisted_tool_policy_entries(&std::collections::HashMap::new());
    assert!(unknown.is_empty(), "mock registry reports no unknown tools");
    let names: Vec<_> = agent
        .current_tool_definitions()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, vec!["alpha".to_string()]);
}

#[test]
fn poisoned_pending_queue_still_queues_and_drains_mutations() {
    let mut agent = make_policy_agent();
    poison(&agent.pending_tool_policy_requests);
    poison(&agent.tool_policy_state);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "cov")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued mutation must survive a poisoned pending queue");
    assert_eq!(reconciliation.results.len(), 1);
    assert_eq!(reconciliation.results[0].name, "alpha");
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );

    // record_applied_tool_policy_overlay recovered the poisoned policy lock,
    // so the disable is visible in the effective definitions.
    assert!(agent.current_tool_definitions().is_empty());

    // Draining again returns None: the queue was emptied despite the poison.
    assert!(agent.drain_tool_policy_mutations_at_boundary().is_none());
}

#[test]
fn notify_tool_catalogue_changed_emits_event_when_catalogue_differs() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("alpha", "ok")));
    let events: Arc<Mutex<Vec<AgentProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |ev| {
        events_clone.lock().unwrap().push(ev);
    });
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        progress_callback: Some(callback),
        ..test_config(provider, Box::new(registry))
    });

    let stale_before = vec![mock_catalogue_entry("ghost", true)];
    agent.notify_tool_catalogue_changed(vec!["ghost".to_string()], stale_before.clone(), "cov");

    let events = events.lock().unwrap();
    match events.as_slice() {
        [
            AgentProgressEvent::ToolCatalogueChanged {
                changed_tools,
                before,
                after,
                reason,
            },
        ] => {
            assert_eq!(changed_tools, &["ghost".to_string()]);
            assert_eq!(before, &stale_before);
            assert!(after.is_empty(), "mock registry exposes no catalogue");
            assert_eq!(reason, "cov");
        }
        other => panic!("expected one ToolCatalogueChanged event, got {other:?}"),
    }
}
