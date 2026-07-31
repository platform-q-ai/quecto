use super::tests::*;
use super::*;
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationResult, ToolPolicyMutationStatus,
    ToolPolicyReconciliation, ToolRegistry,
};
use crate::domain::tool_descriptor::{
    ToolAvailability, ToolCatalogueEntry, ToolHealth, ToolLifecycleKind, ToolSource,
};

fn mock_catalogue_entry(name: &str, effective_enabled: bool) -> ToolCatalogueEntry {
    ToolCatalogueEntry {
        stable_id: name.to_string().into(),
        name: name.to_string().into(),
        label: name.to_string().into(),
        description: "mock".into(),
        input_schema: "{}".into(),
        source: ToolSource::Runtime,
        owner: "mock".into(),
        provider_id: "mock".into(),
        version: None,
        lifecycle: ToolLifecycleKind::RuntimeLoadable,
        configurable: true,
        default_enabled: true,
        configured_enabled: None,
        profile_enabled: None,
        session_enabled: None,
        explicit_restriction: None,
        runtime_availability: if effective_enabled {
            ToolAvailability::Enabled
        } else {
            ToolAvailability::Disabled
        },
        effective_enabled,
        health: if effective_enabled {
            ToolHealth::Ok
        } else {
            ToolHealth::Disabled
        },
    }
}

impl ExtensionToolRegistry for MockRegistry {}
impl SessionAwareTools for MockRegistry {}
impl crate::domain::tool::ToolPolicyMutator for MockRegistry {
    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        let mut results = Vec::new();
        for mutation in mutations {
            let before_enabled = self
                .cached_definitions
                .iter()
                .any(|definition| definition.name.as_ref() == mutation.name);
            if before_enabled && !mutation.availability.is_enabled() {
                self.cached_definitions
                    .retain(|definition| definition.name.as_ref() != mutation.name);
            }
            let after_enabled = self
                .cached_definitions
                .iter()
                .any(|definition| definition.name.as_ref() == mutation.name);
            results.push(ToolPolicyMutationResult {
                name: mutation.name.clone(),
                requested_availability: mutation.availability,
                status: if before_enabled {
                    ToolPolicyMutationStatus::Applied
                } else {
                    ToolPolicyMutationStatus::UnknownTool
                },
                before: before_enabled.then(|| mock_catalogue_entry(&mutation.name, true)),
                after: before_enabled.then(|| mock_catalogue_entry(&mutation.name, after_enabled)),
                reason: mutation.reason.clone(),
            });
        }
        ToolPolicyReconciliation { mode, results }
    }
}
impl ToolRegistry for MockRegistry {}

#[test]
fn queued_tool_policy_mutations_apply_once_at_turn_boundary() {
    let (mut agent, provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    agent.queue_tool_policy_mutation(&[
        ToolPolicyMutation::disable("alpha", "queue first"),
        ToolPolicyMutation::disable("missing", "queue second"),
    ]);
    assert_eq!(
        provider
            .last_tool_defs()
            .iter()
            .map(|definition| definition.name.as_ref())
            .collect::<Vec<_>>(),
        Vec::<&str>::new()
    );

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued policy mutations drain once");
    assert_eq!(reconciliation.mode, ToolPolicyApplyMode::AtNextTurnBoundary);
    assert_eq!(reconciliation.results[0].name, "alpha");
    assert_eq!(reconciliation.results[1].name, "missing");
    assert!(
        reconciliation.results[0]
            .after
            .as_ref()
            .is_some_and(|entry| !entry.effective_enabled)
    );
    assert!(reconciliation.results[1].before.is_none());
    assert!(agent.drain_tool_policy_mutations_at_boundary().is_none());
}

#[test]
fn immediate_if_idle_queues_when_a_turn_is_in_flight() {
    let (mut agent, provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    agent.mark_turn_in_flight();
    let immediate = agent.request_tool_policy_mutation(
        &[ToolPolicyMutation::disable("alpha", "in flight")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert!(immediate.is_none());
    assert_eq!(
        provider
            .last_tool_defs()
            .iter()
            .map(|definition| definition.name.as_ref())
            .collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued in-flight mutation drains at boundary");
    assert_eq!(reconciliation.results[0].name, "alpha");
    assert!(
        reconciliation.results[0]
            .after
            .as_ref()
            .is_some_and(|entry| !entry.effective_enabled)
    );
}

#[tokio::test]
async fn queued_policy_does_not_change_in_flight_turn_tool_manifest() {
    let responses = vec![tool_call_response("alpha", "{}"), text_response("done")];
    let (mut agent, provider) = make_agent(responses, vec![("alpha", "tool output")]);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "next turn")]);
    let mut messages = vec![Message::user("use alpha")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.tool_iterations, 1);
    assert_eq!(
        provider
            .last_tool_defs()
            .iter()
            .map(|definition| definition.name.as_ref())
            .collect::<Vec<_>>(),
        vec!["alpha"]
    );

    agent.drain_tool_policy_mutations_at_boundary().unwrap();
    let mut next_messages = vec![Message::user("next")];
    let _ = agent.run_loop(&mut next_messages).await.unwrap();
    assert!(provider.last_tool_defs().is_empty());
}
