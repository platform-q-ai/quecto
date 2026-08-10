use super::tests::*;
use super::*;
use crate::domain::agent::AgentLoop;
use crate::domain::message::Role;
use crate::domain::tool::{
    RuntimeToolLifecycleRegistry, ToolDefinition, ToolPolicyApplyMode, ToolPolicyMutation,
    ToolPolicyMutationResult, ToolPolicyMutationStatus, ToolPolicyReconciliation,
    ToolProfileContext, ToolRegistry,
};
use crate::domain::tool_descriptor::{
    ProfileAvailabilityScope, ToolAvailability, ToolCatalogueEntry, ToolHealth, ToolLifecycleKind,
    ToolSource,
};

pub(super) fn mock_catalogue_entry(name: &str, effective_enabled: bool) -> ToolCatalogueEntry {
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
        profile_scope: None,
        session_enabled: None,
        explicit_restriction: None,
        runtime_availability: if effective_enabled {
            ToolAvailability::Enabled
        } else {
            ToolAvailability::Disabled
        },
        effective_enabled,
        effective_scope: ProfileAvailabilityScope::from_enabled(effective_enabled),
        effective_parent_enabled: effective_enabled,
        effective_child_enabled: effective_enabled,
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
            let status = if before_exists {
                ToolPolicyMutationStatus::Applied
            } else {
                ToolPolicyMutationStatus::UnknownTool
            };
            if status == ToolPolicyMutationStatus::Applied {
                // Keep the tool definition whenever any profile can see it.
                // Parent/child visibility is enforced by agent-loop scope overlays
                // (and by the real registry's split parent/child caches).
                self.cached_definitions
                    .retain(|definition| definition.name.as_ref() != mutation.name);
                if mutation.scope.is_enabled() {
                    if let Some(tool) = self
                        .tools
                        .iter()
                        .find(|tool| tool.definition().name.as_ref() == mutation.name)
                    {
                        self.cached_definitions.push(tool.definition());
                    } else {
                        self.cached_definitions
                            .push(crate::domain::tool::ToolDefinition {
                                name: mutation.name.to_string().into(),
                                description: format!("Mock {} tool", mutation.name).into(),
                                parameters_schema: r#"{"type":"object"}"#.into(),
                            });
                    }
                }
            }
            let after_enabled = self
                .cached_definitions
                .iter()
                .any(|definition| definition.name.as_ref() == mutation.name);
            results.push(ToolPolicyMutationResult {
                name: mutation.name.clone(),
                requested_identifier: None,
                requested_availability: mutation.availability,
                requested_scope: mutation.scope,
                status,
                before: before_exists.then(|| mock_catalogue_entry(&mutation.name, before_enabled)),
                after: before_exists.then(|| {
                    let mut entry = mock_catalogue_entry(&mutation.name, after_enabled);
                    entry.profile_scope = Some(mutation.scope);
                    entry.profile_enabled = Some(mutation.scope.is_enabled());
                    entry.effective_scope = mutation.scope;
                    entry.effective_parent_enabled = mutation.scope.allows_parent();
                    entry.effective_child_enabled = mutation.scope.allows_child();
                    entry.effective_enabled = mutation.scope.is_enabled();
                    entry.runtime_availability = mutation.availability;
                    entry
                }),
                reason: mutation.reason.clone(),
            });
        }
        ToolPolicyReconciliation {
            mode,
            results,
            correlation_id: None,
        }
    }
}
impl ToolRegistry for MockRegistry {}

pub(super) struct RestrictedMockRegistry {
    inner: MockRegistry,
}

impl RestrictedMockRegistry {
    pub(super) fn new(name: &str) -> Self {
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
            correlation_id: None,
            results: mutations
                .iter()
                .map(|mutation| ToolPolicyMutationResult {
                    name: mutation.name.clone(),
                    requested_identifier: None,
                    requested_availability: mutation.availability,
                    requested_scope: mutation.scope,
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
fn at_next_turn_boundary_queues_even_when_idle() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    let immediate = agent.request_tool_policy_mutation(
        &[ToolPolicyMutation::disable("alpha", "next boundary")],
        ToolPolicyApplyMode::AtNextTurnBoundary,
    );

    assert!(
        immediate.is_none(),
        "atNextTurnBoundary must not apply synchronously while idle"
    );
    assert_eq!(agent.current_tool_definitions()[0].name.as_ref(), "alpha");

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued idle mutation drains at boundary");
    assert_eq!(reconciliation.mode, ToolPolicyApplyMode::AtNextTurnBoundary);
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(agent.current_tool_definitions().is_empty());
}

#[test]
fn at_next_turn_boundary_queues_when_a_turn_is_in_flight() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    agent.mark_turn_in_flight();
    let immediate = agent.request_tool_policy_mutation(
        &[ToolPolicyMutation::disable("alpha", "next boundary")],
        ToolPolicyApplyMode::AtNextTurnBoundary,
    );

    assert!(
        immediate.is_none(),
        "atNextTurnBoundary must queue while busy"
    );
    assert_eq!(agent.current_tool_definitions()[0].name.as_ref(), "alpha");

    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued in-flight mutation drains at boundary");
    assert_eq!(reconciliation.mode, ToolPolicyApplyMode::AtNextTurnBoundary);
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert!(agent.current_tool_definitions().is_empty());
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
async fn queued_policy_drains_at_single_response_final_boundary() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "final boundary")]);
    let mut messages = vec![Message::user("hello")];

    agent
        .process(&mut messages)
        .await
        .expect("single response turn");

    assert!(agent.current_tool_definitions().is_empty());
    assert!(
        agent
            .drain_tool_policy_mutations_at_internal_boundary()
            .is_none(),
        "final boundary must drain queued mutations exactly once"
    );
}

#[tokio::test]
async fn queued_policy_drains_at_terminal_provider_failure_boundary() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        "provider error (401): invalid credentials".to_string(),
    ))]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("alpha", "ok")));
    let mut agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::disable("alpha", "terminal boundary")]);
    let mut messages = vec![Message::user("hello")];

    agent
        .process(&mut messages)
        .await
        .expect_err("provider failure is terminal");

    assert!(agent.current_tool_definitions().is_empty());
    assert!(
        agent
            .drain_tool_policy_mutations_at_internal_boundary()
            .is_none(),
        "terminal boundary must drain queued mutations exactly once"
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
    let (mut agent, _provider) = make_agent(responses, vec![("alpha", "tool output")]);

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
        let mut policy = agent.tool_policy_state.lock().unwrap();
        policy.disabled_tools.insert("alpha".to_string());
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
            .tool_policy_state
            .lock()
            .unwrap()
            .disabled_tools
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

struct CatalogueOnlyRegistry {
    entries: Vec<ToolCatalogueEntry>,
    definitions: Vec<crate::domain::tool::ToolDefinition>,
}

impl ToolCatalog for CatalogueOnlyRegistry {
    fn definitions(&self) -> &[crate::domain::tool::ToolDefinition] {
        &self.definitions
    }

    fn definitions_for(&self, context: ToolProfileContext) -> &[ToolDefinition] {
        match context {
            ToolProfileContext::Parent => &self.definitions,
            ToolProfileContext::Child => &self.definitions,
        }
    }

    fn catalogue_entries(&self) -> Vec<ToolCatalogueEntry> {
        self.entries.clone()
    }
}

impl ToolExecutor for CatalogueOnlyRegistry {
    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::domain::tool::ToolResult, DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { Err(DomainError::Tool("not implemented".into())) })
    }
}

impl RuntimeToolLifecycleRegistry for CatalogueOnlyRegistry {}
impl SessionAwareTools for CatalogueOnlyRegistry {}
impl crate::domain::tool::ToolPolicyMutator for CatalogueOnlyRegistry {}
impl ToolRegistry for CatalogueOnlyRegistry {}

#[test]
fn tool_catalogue_entries_apply_runtime_policy_overlay() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let registry = CatalogueOnlyRegistry {
        entries: vec![mock_catalogue_entry("alpha", true)],
        definitions: vec![],
    };
    let agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));
    {
        let mut policy = agent.tool_policy_state.lock().unwrap();
        policy.record_applied("alpha", ProfileAvailabilityScope::Child);
    }

    let entry = agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "alpha")
        .expect("alpha catalogue entry");

    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::Child);
    assert!(!entry.effective_parent_enabled);
    assert!(entry.effective_child_enabled);
}

#[test]
fn current_tool_definitions_hide_child_only_scope_from_parent_requests() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut entry = mock_catalogue_entry("child_only", true);
    entry.profile_enabled = Some(true);
    entry.profile_scope = Some(ProfileAvailabilityScope::Child);
    entry.effective_scope = ProfileAvailabilityScope::Child;
    entry.effective_parent_enabled = false;
    entry.effective_child_enabled = true;
    let registry = CatalogueOnlyRegistry {
        entries: vec![entry],
        definitions: vec![],
    };
    let agent = AgentLoopImpl::new(test_config(provider, Box::new(registry)));

    assert!(agent.current_tool_definitions().is_empty());
}

#[tokio::test]
async fn first_turn_uses_configured_profile_for_model_visible_tools() {
    let provider = Arc::new(MockProvider::new(vec![text_response("done")]));
    let mut spawn = mock_catalogue_entry("spawn", true);
    spawn.profile_scope = Some(ProfileAvailabilityScope::Parent);
    spawn.effective_scope = ProfileAvailabilityScope::Parent;
    spawn.effective_parent_enabled = true;
    spawn.effective_child_enabled = false;
    let mut docs = mock_catalogue_entry("docs", true);
    docs.profile_scope = Some(ProfileAvailabilityScope::Child);
    docs.effective_scope = ProfileAvailabilityScope::Child;
    docs.effective_parent_enabled = false;
    docs.effective_child_enabled = true;
    let registry = CatalogueOnlyRegistry {
        entries: vec![spawn, docs],
        definitions: vec![],
    };
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        tool_profile_context: ToolProfileContext::Child,
        ..test_config(provider.clone(), Box::new(registry))
    });
    let mut messages = vec![Message::user("hello")];

    agent.process(&mut messages).await.expect("first turn");
    let names: Vec<_> = provider
        .last_tool_defs()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert!(names.iter().any(|name| name.as_ref() == "docs"));
    assert!(!names.iter().any(|name| name.as_ref() == "spawn"));
}

#[tokio::test]
async fn first_turn_parent_profile_keeps_parent_tools_model_visible() {
    let provider = Arc::new(MockProvider::new(vec![text_response("done")]));
    let mut spawn = mock_catalogue_entry("spawn", true);
    spawn.profile_scope = Some(ProfileAvailabilityScope::Parent);
    spawn.effective_scope = ProfileAvailabilityScope::Parent;
    spawn.effective_parent_enabled = true;
    spawn.effective_child_enabled = false;
    let registry = CatalogueOnlyRegistry {
        entries: vec![spawn],
        definitions: vec![],
    };
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        tool_profile_context: ToolProfileContext::Parent,
        ..test_config(provider.clone(), Box::new(registry))
    });
    let mut messages = vec![Message::user("hello")];

    agent.process(&mut messages).await.expect("first turn");
    assert!(
        provider
            .last_tool_defs()
            .iter()
            .any(|definition| definition.name.as_ref() == "spawn")
    );
}

#[tokio::test]
async fn direct_execution_honors_runtime_profile_scope() {
    for (scope, context, should_execute) in [
        (
            ProfileAvailabilityScope::Parent,
            ToolProfileContext::Parent,
            true,
        ),
        (
            ProfileAvailabilityScope::Parent,
            ToolProfileContext::Child,
            false,
        ),
        (
            ProfileAvailabilityScope::Child,
            ToolProfileContext::Parent,
            false,
        ),
        (
            ProfileAvailabilityScope::Child,
            ToolProfileContext::Child,
            true,
        ),
        (
            ProfileAvailabilityScope::Both,
            ToolProfileContext::Parent,
            true,
        ),
        (
            ProfileAvailabilityScope::Both,
            ToolProfileContext::Child,
            true,
        ),
        (
            ProfileAvailabilityScope::None,
            ToolProfileContext::Parent,
            false,
        ),
        (
            ProfileAvailabilityScope::None,
            ToolProfileContext::Child,
            false,
        ),
    ] {
        let provider = Arc::new(MockProvider::new(vec![]));
        let mut registry = MockRegistry::new();
        registry.register(Arc::new(MockTool::new("alpha", "executed")));
        let mut agent = AgentLoopImpl::new(AgentLoopConfig {
            tool_profile_context: context,
            ..test_config(provider, Box::new(registry))
        });
        agent
            .request_tool_policy_mutation(
                &[ToolPolicyMutation::set_scope("alpha", scope, "scope test")],
                ToolPolicyApplyMode::ImmediateIfIdle,
            )
            .expect("policy applies");

        let mut messages = Vec::new();
        let mut run_ledger = Vec::new();
        agent
            .execute_tool_calls_for_response(
                &mut messages,
                1,
                tool_call_response("alpha", "{}"),
                &mut run_ledger,
            )
            .await;
        let tool_result = messages.last().expect("tool result message");

        if should_execute {
            assert_eq!(
                tool_result.content, "executed",
                "{scope:?} in {context:?} should execute"
            );
            assert!(!tool_result.is_error);
        } else {
            assert!(
                tool_result.content.contains("disabled by runtime policy"),
                "{scope:?} in {context:?} should be blocked, got {:?}",
                tool_result.content
            );
            assert!(tool_result.is_error);
        }
    }
}

#[path = "agent_loop_policy_drain_tests.rs"]
mod drain_tests;
