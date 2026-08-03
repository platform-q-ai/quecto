use super::super::tests::*;
use super::super::*;
use super::RestrictedMockRegistry;
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use std::sync::Arc;

#[test]
fn queued_drain_keeps_registry_catalogue_and_event_after_consistent() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let events_cb = events.clone();
    agent.set_progress_callback(Some(Arc::new(move |event| {
        events_cb.lock().unwrap().push(event);
    })));

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "queue disable")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued disable drains");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    let after = reconciliation.results[0]
        .after
        .as_ref()
        .expect("applied mutation includes after snapshot");
    assert!(!after.effective_enabled);
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::None);

    // Registry path and event snapshot must agree: definitions hide disabled tools,
    // overlay records Applied mutations, and event `after` matches reconciliation.
    assert!(
        agent.tool_registry.definitions().is_empty(),
        "registry definitions must hide tools disabled via queued drain"
    );
    assert!(agent.current_tool_definitions().is_empty());
    assert!(
        agent
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .contains("alpha"),
        "overlay records only Applied mutations"
    );

    let events = events.lock().unwrap();
    let changed = events.iter().find_map(|event| match event {
        crate::domain::agent::AgentProgressEvent::ToolPolicyChanged { reconciliation, .. } => {
            Some(reconciliation)
        }
        _ => None,
    });
    let event_after = changed
        .expect("turn_boundary emits tool_policy_changed")
        .results[0]
        .after
        .as_ref()
        .expect("event carries after snapshot");
    assert_eq!(event_after.effective_enabled, after.effective_enabled);
    assert_eq!(event_after.effective_scope, after.effective_scope);
}

#[test]
fn blocked_queued_mutation_does_not_mutate_overlays() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    {
        let mut disabled = agent.runtime_disabled_tools.lock().unwrap();
        disabled.insert("alpha".to_string());
    }
    agent.swap_registry(Box::new(RestrictedMockRegistry::new("alpha")));
    {
        let mut enabled = agent.runtime_enabled_tools.lock().unwrap();
        enabled.insert("alpha".to_string());
        let mut scopes = agent.runtime_policy_scopes.lock().unwrap();
        scopes.insert("alpha".to_string(), ProfileAvailabilityScope::Both);
    }

    let disabled_before = agent.runtime_disabled_tools.lock().unwrap().clone();
    let enabled_before = agent.runtime_enabled_tools.lock().unwrap().clone();
    let scopes_before = agent.runtime_policy_scopes.lock().unwrap().clone();

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::enable("alpha", "blocked enable")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("blocked queued mutation still drains a result");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert_eq!(
        *agent.runtime_disabled_tools.lock().unwrap(),
        disabled_before,
        "blocked queued mutations must not rewrite disabled overlay"
    );
    assert_eq!(
        *agent.runtime_enabled_tools.lock().unwrap(),
        enabled_before,
        "blocked queued mutations must not rewrite enabled overlay"
    );
    assert_eq!(
        *agent.runtime_policy_scopes.lock().unwrap(),
        scopes_before,
        "blocked queued mutations must not rewrite scope overlay"
    );
}

#[test]
fn immediate_child_scope_mutation_does_not_enter_parent_enabled_overlay() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    let reconciliation = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::set_scope(
                "alpha",
                ProfileAvailabilityScope::Child,
                "child only",
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("immediate scope applies");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(
        !agent
            .runtime_enabled_tools
            .lock()
            .unwrap()
            .contains("alpha")
    );
    assert!(
        agent
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .contains("alpha")
    );
    assert!(agent.current_tool_definitions().is_empty());

    let after = reconciliation.results[0].after.as_ref().unwrap();
    assert!(after.effective_child_enabled);
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::Child);
}

#[test]
fn queued_child_scope_mutation_reports_and_applies_child_scope() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::Child,
        "child later",
    )]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_internal_boundary()
        .expect("queued child scope drains");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    let after = reconciliation.results[0].after.as_ref().unwrap();
    assert_eq!(after.profile_scope, Some(ProfileAvailabilityScope::Child));
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::Child);
    assert!(!after.effective_parent_enabled);
    assert!(after.effective_child_enabled);
    assert!(
        !agent
            .runtime_enabled_tools
            .lock()
            .unwrap()
            .contains("alpha")
    );
    assert!(
        agent
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .contains("alpha")
    );
    assert!(agent.current_tool_definitions().is_empty());

    let mut child_agent = agent;
    child_agent.tool_profile_context = ToolProfileContext::Child;
    let child_defs = child_agent.current_tool_definitions();
    assert_eq!(child_defs.len(), 1);
    assert_eq!(child_defs[0].name.as_ref(), "alpha");
}

#[test]
fn parent_only_applied_mutation_stays_hidden_on_child_profile_agent() {
    // Parent-only Applied overlays must not leak into Child-profile model-visible
    // definitions via runtime_enabled_tools (which previously OR'd past scope).
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    agent.tool_profile_context = ToolProfileContext::Child;

    let reconciliation = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::set_scope(
                "alpha",
                ProfileAvailabilityScope::Parent,
                "parent only",
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("immediate parent-only scope applies");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    let after = reconciliation.results[0]
        .after
        .as_ref()
        .expect("applied mutation includes after snapshot");
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::Parent);
    assert!(after.effective_parent_enabled);
    assert!(!after.effective_child_enabled);

    // Scope overlay records Parent, but the enabled overlay must not force the
    // tool into Child-profile catalogues / definitions.
    assert_eq!(
        agent
            .runtime_policy_scopes
            .lock()
            .unwrap()
            .get("alpha")
            .copied(),
        Some(ProfileAvailabilityScope::Parent)
    );
    assert!(
        !agent
            .runtime_enabled_tools
            .lock()
            .unwrap()
            .contains("alpha"),
        "parent-only Applied must not enter runtime_enabled_tools"
    );
    assert!(
        agent.current_tool_definitions().is_empty(),
        "child-profile agent must hide parent-only tools even after Applied overlay"
    );

    // Parent-profile agents still see Parent-only tools via the scope overlay.
    agent.tool_profile_context = ToolProfileContext::Parent;
    let parent_defs = agent.current_tool_definitions();
    assert_eq!(parent_defs.len(), 1);
    assert_eq!(parent_defs[0].name.as_ref(), "alpha");
}

#[test]
fn both_scope_applied_mutation_enters_enabled_overlay_and_stays_visible() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    // Start from a disabled tool so Both is a real Applied transition (not AlreadyInState).
    let disabled = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::disable("alpha", "start disabled")],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("immediate disable applies");
    assert_eq!(
        disabled.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(agent.current_tool_definitions().is_empty());

    let reconciliation = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::set_scope(
                "alpha",
                ProfileAvailabilityScope::Both,
                "both profiles",
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("immediate both scope applies");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(
        agent
            .runtime_enabled_tools
            .lock()
            .unwrap()
            .contains("alpha"),
        "Both Applied may enter runtime_enabled_tools"
    );
    assert_eq!(agent.current_tool_definitions()[0].name.as_ref(), "alpha");

    agent.tool_profile_context = ToolProfileContext::Child;
    assert_eq!(agent.current_tool_definitions()[0].name.as_ref(), "alpha");
}
