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
