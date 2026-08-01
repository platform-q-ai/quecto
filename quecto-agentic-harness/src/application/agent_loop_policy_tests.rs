use super::tests::*;
use super::*;
use crate::domain::message::Role;
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

impl RuntimeToolLifecycleRegistry for MockRegistry {}
impl SessionAwareTools for MockRegistry {}
impl crate::domain::tool::ToolPolicyMutator for MockRegistry {
    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        let mut results = Vec::new();
        for mutation in mutations {
            let before_exists = self
                .tools
                .iter()
                .any(|tool| tool.definition().name.as_ref() == mutation.name);
            let before_enabled = self
                .cached_definitions
                .iter()
                .any(|definition| definition.name.as_ref() == mutation.name);
            if before_enabled && !mutation.availability.is_enabled() {
            } else if !before_enabled && mutation.availability.is_enabled() {
                self.cached_definitions
                    .push(crate::domain::tool::ToolDefinition {
                        name: mutation.name.to_string().into(),
                        description: format!("Mock {} tool", mutation.name).into(),
                        parameters_schema: r#"{"type":"object"}"#.into(),
                    });
            }
            let after_enabled = self
                .cached_definitions
                .iter()
                .any(|definition| definition.name.as_ref() == mutation.name);
            results.push(ToolPolicyMutationResult {
                name: mutation.name.clone(),
                requested_availability: mutation.availability,
                status: if before_exists {
                    ToolPolicyMutationStatus::Applied
                } else {
                    ToolPolicyMutationStatus::UnknownTool
                },
                before: before_exists.then(|| mock_catalogue_entry(&mutation.name, before_enabled)),
                after: before_exists.then(|| mock_catalogue_entry(&mutation.name, after_enabled)),
                reason: mutation.reason.clone(),
            });
        }
        ToolPolicyReconciliation { mode, results }
    }
}
impl ToolRegistry for MockRegistry {}

struct RestrictedMockRegistry {
    inner: MockRegistry,
}

impl RestrictedMockRegistry {
    fn new(name: &str) -> Self {
        let mut inner = MockRegistry::new();
        inner.register(Arc::new(MockTool::new(name, "ok")));
        Self { inner }
    }
}

impl ToolCatalog for RestrictedMockRegistry {
    fn definitions(&self) -> &[crate::domain::tool::ToolDefinition] {
        self.inner.definitions()
    }

    fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        let mut entry = mock_catalogue_entry("alpha", false);
        entry.runtime_availability = ToolAvailability::Disabled;
        entry.explicit_restriction =
            Some(crate::domain::tool_descriptor::ToolRestrictionReason::Spawn);
        vec![entry]
    }
}

impl ToolExecutor for RestrictedMockRegistry {
    fn execute(
        &self,
        name: &str,
        arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::domain::tool::ToolResult, DomainError>>
                + Send
                + '_,
        >,
    > {
        self.inner.execute(name, arguments)
    }
}

impl RuntimeToolLifecycleRegistry for RestrictedMockRegistry {}
impl SessionAwareTools for RestrictedMockRegistry {}
impl crate::domain::tool::ToolPolicyMutator for RestrictedMockRegistry {
    fn apply_tool_policy_mutations(
        &mut self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> ToolPolicyReconciliation {
        ToolPolicyReconciliation {
            mode,
            results: mutations
                .iter()
                .map(|mutation| ToolPolicyMutationResult {
                    name: mutation.name.clone(),
                    requested_availability: mutation.availability,
                    status: ToolPolicyMutationStatus::BlockedByRestriction,
                    before: Some(self.catalogue_entries()[0].clone()),
                    after: Some(self.catalogue_entries()[0].clone()),
                    reason: mutation.reason.clone(),
                })
                .collect(),
        }
    }
}
impl ToolRegistry for RestrictedMockRegistry {}

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
    let (agent, provider) = make_agent(responses, vec![("alpha", "tool output")]);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "next turn")]);
    let mut messages = vec![Message::user("use alpha")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.tool_iterations, 1);
    assert!(provider.last_tool_defs().is_empty());
}

#[tokio::test]
async fn queued_policy_change_event_emits_from_internal_turn_boundary() {
    let responses = vec![tool_call_response("alpha", "{}"), text_response("done")];
    let (mut agent, _provider) = make_agent(responses, vec![("alpha", "tool output")]);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    agent.set_progress_callback(Some(Arc::new(move |event| {
        if matches!(event, AgentProgressEvent::ToolPolicyChanged { .. }) {
            captured.lock().unwrap().push(event);
        }
    })));
    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "next turn")]);
    let mut messages = vec![Message::user("use alpha")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.tool_iterations, 1);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn queued_policy_disable_blocks_stale_post_boundary_tool_call() {
    let responses = vec![
        tool_call_response("alpha", "{}"),
        tool_call_response("alpha", "{}"),
        text_response("done"),
    ];
    let (agent, _provider) = make_agent(responses, vec![("alpha", "tool output")]);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "next turn")]);
    let mut messages = vec![Message::user("use alpha")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.tool_iterations, 2);

    let stale_result = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Tool)
        .expect("stale tool result appended");
    assert!(
        stale_result.content.contains("disabled by runtime policy"),
        "stale post-boundary call must be rejected: {}",
        stale_result.content
    );
}

#[test]
fn queued_policy_enable_preserves_restricted_status() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    {
        let mut disabled = agent.runtime_disabled_tools.lock().unwrap();
        disabled.insert("alpha".to_string());
    }
    agent.swap_registry(Box::new(RestrictedMockRegistry::new("alpha")));

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::enable("alpha", "try enable")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued enable drains");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert!(
        agent
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .contains("alpha")
    );
}

#[test]
fn queued_policy_enable_restores_registry_disabled_tool_manifest() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let disabled = agent
        .request_tool_policy_mutation(
            &[ToolPolicyMutation::disable("alpha", "disable now")],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("immediate disable applies");
    assert_eq!(
        disabled.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(agent.current_tool_definitions().is_empty());

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::enable("alpha", "enable later")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued enable drains");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert_eq!(agent.current_tool_definitions()[0].name.as_ref(), "alpha");
}
