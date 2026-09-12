//! Find reaches the concrete effect through the production registry runtime.
use super::tool_runtime::*;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot;

fn runtime(
    root: &std::path::Path,
    entrypoint: ToolEntrypoint,
    profile_context: ToolRuntimeProfileContext,
    restriction: &str,
) -> ToolRuntimeBuild {
    let mut config = Config::default();
    if matches!(restriction, "persisted" | "profile") {
        config.tools.policy.entries.insert(
            crate::domain::tool_id::stable_tool_id(
                crate::domain::tool_descriptor::ToolSource::BundledNative,
                "quecto:official-tools",
                "find",
            ),
            ToolPolicyEntryConfig {
                scope: if restriction == "profile" {
                    ProfileAvailabilityScope::Parent
                } else {
                    ProfileAvailabilityScope::None
                },
            },
        );
    }
    let inherited = (restriction == "inherited").then(|| InheritedToolPolicySnapshot {
        version: 1,
        tools: Default::default(),
    });
    let disabled = if restriction == "disabled" {
        vec!["find".into()]
    } else {
        vec![]
    };
    let client = reqwest::Client::new();
    let mut stderr = String::new();
    build_tool_runtime(ToolRuntimeBuildArgs {
        swarm_context: None,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        entrypoint,
        profile_context,
        base_dir: root,
        config: &config,
        http_client: &client,
        workspace: root.to_path_buf(),
        sandbox: Sandbox::new(Some(root.to_path_buf())),
        exec_options: Default::default(),
        session_key: "find-runtime".into(),
        spawned: false,
        parent_session_name: None,
        parent_config_path: None,
        disabled_tools: &disabled,
        inherited_tool_policy: inherited,
        workflow: ToolRuntimeWorkflowPolicy::disabled(root, Some(root)),
        stderr: &mut stderr,
    })
    .expect("production runtime builds")
}

#[tokio::test]
async fn find_executes_through_cli_and_uds_production_runtime() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("runtime-proof.rs"), "").unwrap();
    for entrypoint in [ToolEntrypoint::CliAgent, ToolEntrypoint::UdsAgent] {
        for profile in [
            ToolRuntimeProfileContext::Parent,
            ToolRuntimeProfileContext::Child,
        ] {
            let built = runtime(root.path(), entrypoint, profile, "allowed");
            let result = built
                .registry
                .execute("find", r#"{"pattern":"runtime-proof.rs"}"#)
                .await
                .unwrap();
            assert!(!result.is_error);
            assert_eq!(result.content, "runtime-proof.rs");
            let missing = built.registry.execute("find", "").await.unwrap();
            assert!(missing.is_error);
            assert!(missing.content.contains("pattern"));
        }
    }
}

#[tokio::test]
async fn find_generic_runtime_policy_refuses_before_search() {
    let root = tempfile::tempdir().unwrap();
    for restriction in ["disabled", "persisted", "inherited", "profile"] {
        let built = runtime(
            root.path(),
            ToolEntrypoint::UdsAgent,
            ToolRuntimeProfileContext::Child,
            restriction,
        );
        // Valid syntax with a nonexistent search root: a backend invocation would
        // yield an fd search diagnostic rather than this shared policy refusal.
        let result = built
            .registry
            .execute("find", r#"{"pattern":"*","path":"missing-root"}"#)
            .await
            .unwrap();
        assert!(result.is_error, "{restriction}");
        assert!(
            result.content.contains("disabled") || result.content.contains("available"),
            "{restriction}: {}",
            result.content
        );
    }
}
