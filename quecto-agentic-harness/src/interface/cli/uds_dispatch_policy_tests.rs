use super::*;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::interface::cli::protocol::{AgentCommand, ToolPolicyApplyModeCommand};

#[derive(Debug)]
struct NamedTool(&'static str);

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
        operation: crate::interface::cli::protocol::ToolPolicyOperationCommand::Patch,
        unlisted_scope: None,
        persist: false,
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
async fn dispatch_set_tool_policy_prefers_tool_id_when_name_also_present() {
    let mut fx = cov_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("beta")));
    let beta_id = fx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "beta")
        .expect("beta entry")
        .stable_id
        .into_owned();

    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: Some(beta_id),
            name: Some("alpha".into()),
            scope: ProfileAvailabilityScope::Child,
            reason: Some("test".into()),
        }],
        mode: ToolPolicyApplyModeCommand::ImmediateIfIdle,
        operation: crate::interface::cli::protocol::ToolPolicyOperationCommand::Patch,
        unlisted_scope: None,
        persist: false,
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }

    let entries = fx.agent.tool_catalogue_entries();
    let alpha = entries
        .iter()
        .find(|entry| entry.name == "alpha")
        .expect("alpha entry");
    let beta = entries
        .iter()
        .find(|entry| entry.name == "beta")
        .expect("beta entry");
    assert_eq!(alpha.profile_scope, None);
    assert_eq!(alpha.effective_scope, ProfileAvailabilityScope::Both);
    assert_eq!(beta.profile_scope, Some(ProfileAvailabilityScope::Child));
    assert_eq!(beta.effective_scope, ProfileAvailabilityScope::Child);
}

#[tokio::test]
async fn dispatch_set_tool_policy_tool_id_only_still_applies() {
    let mut fx = cov_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    let alpha_id = fx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "alpha")
        .expect("alpha entry")
        .stable_id
        .into_owned();

    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: Some(alpha_id),
            name: None,
            scope: ProfileAvailabilityScope::Child,
            reason: Some("test".into()),
        }],
        mode: ToolPolicyApplyModeCommand::ImmediateIfIdle,
        operation: crate::interface::cli::protocol::ToolPolicyOperationCommand::Patch,
        unlisted_scope: None,
        persist: false,
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
}
