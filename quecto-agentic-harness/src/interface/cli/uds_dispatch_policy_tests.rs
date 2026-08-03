use super::*;
use crate::domain::tool::{
    ChildToolPolicyPropagation, ChildToolPolicyPropagationStatus, Tool, ToolDefinition,
    ToolPolicyApplyMode, ToolPolicyChildPropagator, ToolPolicyMutation, ToolResult,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::interface::cli::protocol::{AgentCommand, ToolPolicyApplyModeCommand};

#[derive(Debug)]
struct NamedTool(&'static str);

#[derive(Default)]
struct RecordingDispatchPropagator {
    calls: std::sync::Mutex<Vec<(Vec<String>, ToolPolicyApplyMode)>>,
}

impl RecordingDispatchPropagator {
    fn calls(&self) -> Vec<(Vec<String>, ToolPolicyApplyMode)> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ToolPolicyChildPropagator for RecordingDispatchPropagator {
    fn has_children(&self) -> bool {
        true
    }

    fn propagate_tool_policy_to_children(
        &self,
        mutations: &[ToolPolicyMutation],
        mode: ToolPolicyApplyMode,
    ) -> Vec<ChildToolPolicyPropagation> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((
                mutations
                    .iter()
                    .map(|mutation| mutation.name.clone())
                    .collect(),
                mode,
            ));
        vec![ChildToolPolicyPropagation {
            agent_id: "child-1".to_string(),
            status: ChildToolPolicyPropagationStatus::Queued,
            reconciliation: None,
            error: None,
        }]
    }
}

impl Tool for NamedTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.0.into(),
            description: "test".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, crate::domain::error::DomainError>>
                + Send,
        >,
    > {
        Box::pin(async {
            Ok(ToolResult {
                content: "ok".into(),
                is_error: false,
                image_blocks: Vec::new(),
            })
        })
    }
}

#[tokio::test]
async fn dispatch_set_tool_policy_applies_and_catalogue_reflects_scope() {
    let mut fx = cov_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ProfileAvailabilityScope::Child,
            reason: Some("test".into()),
        }],
        mode: ToolPolicyApplyModeCommand::ImmediateIfIdle,
        propagated: false,
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    let alpha = fx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "alpha")
        .expect("alpha entry");
    assert_eq!(alpha.profile_scope, Some(ProfileAvailabilityScope::Child));
    assert_eq!(alpha.effective_scope, ProfileAvailabilityScope::Child);
    assert!(!alpha.effective_parent_enabled);
    assert!(alpha.effective_child_enabled);
}

#[tokio::test]
async fn dispatch_set_tool_policy_immediate_if_idle_applies_and_propagates_when_parent_idle() {
    let mut fx = cov_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    let propagator = std::sync::Arc::new(RecordingDispatchPropagator::default());
    fx.agent.tool_policy_child_propagator = Some(propagator.clone());
    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ProfileAvailabilityScope::None,
            reason: Some("tui tool policy modal".into()),
        }],
        mode: ToolPolicyApplyModeCommand::ImmediateIfIdle,
        propagated: false,
    };

    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    assert_eq!(
        propagator.calls(),
        vec![(
            vec!["alpha".to_string()],
            ToolPolicyApplyMode::ImmediateIfIdle
        )]
    );
    assert!(
        fx.agent.drain_tool_policy_mutations_at_boundary().is_none(),
        "idle parent ImmediateIfIdle modal changes apply immediately without forcing a later turn"
    );
    let alpha = fx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "alpha")
        .expect("alpha entry");
    assert_eq!(alpha.effective_scope, ProfileAvailabilityScope::None);
}
