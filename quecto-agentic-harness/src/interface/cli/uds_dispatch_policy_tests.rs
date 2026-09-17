use super::*;
use crate::application::tools::ports::Tool;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};
use crate::interface::cli::catalogue_handles::RuntimeConfigurationInputs;
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
                delivery_metadata: None,
            })
        })
    }
}

#[tokio::test]
async fn dispatch_set_tool_policy_applies_and_catalogue_reflects_scope() {
    let mut fx = fixture_tests::Fixture::new();
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
    let mut fx = fixture_tests::Fixture::new();
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
    let mut fx = fixture_tests::Fixture::new();
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

#[tokio::test]
async fn immediate_persist_failure_dispatch_returns_error_without_retained_policy() {
    let mut fx = fixture_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    let tmp = tempfile::TempDir::new().unwrap();
    fx.runtime_configuration = Some(RuntimeConfigurationInputs {
        config_path: tmp.path().to_path_buf(),
        env_overrides: std::collections::HashMap::new(),
        http_client: reqwest::Client::new(),
    });

    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ProfileAvailabilityScope::Child,
            reason: Some("durable".into()),
        }],
        mode: ToolPolicyApplyModeCommand::ImmediateIfIdle,
        operation: crate::interface::cli::protocol::ToolPolicyOperationCommand::Patch,
        unlisted_scope: None,
        persist: true,
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
    assert_eq!(alpha.configured_enabled, None);
}

#[tokio::test]
async fn forced_reload_reapplies_persisted_tool_policy_to_live_registry() {
    let mut fx = fixture_tests::Fixture::new();
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
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    let mut config = Config::default();
    config.tools.policy.entries.insert(
        alpha_id,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::None,
        },
    );
    config.providers.openai.api_key = "test-key".into();
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    fx.runtime_configuration = Some(RuntimeConfigurationInputs {
        config_path,
        env_overrides: std::collections::HashMap::new(),
        http_client: reqwest::Client::new(),
    });

    {
        let mut ctx = fx.ctx();
        assert!(
            !dispatch_command(
                AgentCommand::Reload {
                    id: Some("reload".into())
                },
                &mut ctx
            )
            .await
        );
    }

    let alpha = fx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "alpha")
        .expect("alpha entry");
    assert_eq!(alpha.configured_enabled, Some(false));
    assert_eq!(alpha.profile_scope, Some(ProfileAvailabilityScope::None));
    assert!(!alpha.effective_enabled);
}

#[tokio::test]
async fn queued_persist_tool_policy_is_written_when_boundary_drains() {
    let mut fx = fixture_tests::Fixture::new();
    fx.agent
        .register_runtime_tool(std::sync::Arc::new(NamedTool("alpha")));
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&Config::default()).unwrap(),
    )
    .unwrap();
    fx.runtime_configuration = Some(RuntimeConfigurationInputs {
        config_path: config_path.clone(),
        env_overrides: std::collections::HashMap::new(),
        http_client: reqwest::Client::new(),
    });

    let cmd = AgentCommand::SetToolPolicy {
        id: Some("pol".into()),
        mutations: vec![crate::interface::cli::protocol::ToolPolicyMutationCommand {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ProfileAvailabilityScope::Child,
            reason: Some("durable".into()),
        }],
        mode: ToolPolicyApplyModeCommand::AtNextTurnBoundary,
        operation: crate::interface::cli::protocol::ToolPolicyOperationCommand::Patch,
        unlisted_scope: None,
        persist: true,
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }

    assert!(
        Config::load(config_path.to_str().unwrap())
            .unwrap()
            .tools
            .policy
            .entries
            .is_empty(),
        "queued requests must persist after the boundary applies, not before"
    );

    fx.agent.drain_tool_policy_mutations_at_boundary();

    let config = Config::load(config_path.to_str().unwrap()).unwrap();
    assert!(
        config
            .tools
            .policy
            .entries
            .values()
            .any(|entry| entry.scope == ProfileAvailabilityScope::Child),
        "expected queued persisted policy entry in config, got {:?}",
        config.tools.policy.entries
    );
}
