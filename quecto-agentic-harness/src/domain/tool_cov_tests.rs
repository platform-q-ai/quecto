use super::*;
use crate::domain::workflow::{WorkflowConfig, WorkflowEngine};
use crate::infrastructure::extensions::uds_tool::UdsExtensionTool;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::bash::ExecTool;
use crate::infrastructure::tools::docs::DocsTool;
use crate::infrastructure::tools::filesystem::{EditTool, LsTool, ReadTool, WriteTool};
use crate::infrastructure::tools::find::FindTool;
use crate::infrastructure::tools::grep::GrepTool;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_fetch::WebFetchTool;
use crate::infrastructure::tools::web_search::WebSearchTool;
use crate::infrastructure::tools::workflow_tool::WorkflowTool;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn sandboxed_tools() -> (tempfile::TempDir, Arc<PathBuf>, Arc<Sandbox>) {
    // A per-test TempDir: a fixed shared path under the system temp dir collides
    // across concurrent and repeated runs.
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = Arc::new(dir.path().to_path_buf());
    let sandbox = Arc::new(Sandbox::new(Some((*workspace).clone())));
    (dir, workspace, sandbox)
}

#[test]
fn concrete_tools_without_session_state_accept_default_set_session_key() {
    let (_workspace_dir, workspace, sandbox) = sandboxed_tools();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let workflow_engine = Arc::new(Mutex::new(
        WorkflowEngine::new(
            WorkflowConfig {
                auto_continue: true,
                completion_nudge: true,
                selector_prompt: None,
                dir: None,
                templates: vec![],
            },
            false,
        )
        .expect("default workflow templates are valid"),
    ));

    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(UdsExtensionTool::new(
            ToolDefinition {
                name: "ext_tool".into(),
                description: "extension".into(),
                parameters_schema: "{}".into(),
            },
            tx,
            Duration::from_millis(1),
        )),
        Box::new(LsTool::new(workspace.clone(), sandbox.clone())),
        Box::new(EditTool::new(workspace.clone(), sandbox.clone())),
        Box::new(ReadTool::new(workspace.clone(), sandbox.clone())),
        Box::new(WriteTool::new(workspace.clone(), sandbox.clone())),
        Box::new(WebSearchTool::new(None)),
        Box::new(WorkflowTool::new(workflow_engine)),
        Box::new(ExecTool::new(workspace.clone(), sandbox.clone())),
        Box::new(DocsTool::new()),
        Box::new(FindTool::new(workspace.clone(), sandbox.clone())),
        Box::new(GrepTool::new(workspace, sandbox)),
        Box::new(SpawnTool::new(vec!["child".to_string()])),
        Box::new(AgentCmdTool::new(AgentCmdTool::new_registry())),
        Box::new(WebFetchTool::with_client(reqwest::Client::new(), 1)),
    ];

    let names: Vec<String> = tools
        .iter()
        .map(|tool| {
            tool.set_session_key("session-key-covered".to_string());
            tool.definition().name.to_string()
        })
        .collect();

    assert_eq!(
        names,
        vec![
            "ext_tool",
            "ls",
            "edit",
            "read",
            "write",
            "web_search",
            "workflow",
            "bash",
            "docs",
            "find",
            "grep",
            "spawn",
            "agent_cmd",
            "web_fetch",
        ]
    );
}

struct CovNoopTool;

impl Tool for CovNoopTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "cov_noop".into(),
            description: "".into(),
            parameters_schema: "{}".into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: "ok".into(),
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

struct CovEmptyRegistry;

impl ToolCatalog for CovEmptyRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &[]
    }
}

impl ToolExecutor for CovEmptyRegistry {
    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("missing".into())) })
    }
}

impl RuntimeToolLifecycleRegistry for CovEmptyRegistry {}

impl SessionAwareTools for CovEmptyRegistry {}

impl ToolPolicyMutator for CovEmptyRegistry {}

impl ToolRegistry for CovEmptyRegistry {}

#[test]
fn empty_registry_default_owner_cleanup_returns_no_removed_tools() {
    let mut registry = CovEmptyRegistry;
    assert!(
        registry
            .unregister_extensions_for_owner("uds:client:missing")
            .is_empty()
    );
}

#[tokio::test]
async fn local_tool_and_registry_execute_trait_surface() {
    let tool = CovNoopTool;
    Tool::set_session_key(&tool, "covered".into());
    assert_eq!(Tool::definition(&tool).name, "cov_noop");
    assert_eq!(tool.execute("{}").await.expect("tool ok").content, "ok");

    let mut registry = CovEmptyRegistry;
    assert_eq!(registry.tool_count(), 0);
    assert!(registry.runtime_tool_names().is_empty());
    assert!(registry.can_register_uds_tool_for_owner("cov_noop", "uds:client:cov"));
    assert!(registry.can_register_uds_tool_for_owner_with_stable_id(
        "cov_noop",
        "uds:client:cov",
        Some("com.example.cov-noop.v1"),
    ));
    assert!(
        !registry.register_uds_tool_for_owner(
            std::sync::Arc::new(CovNoopTool),
            "uds:client:cov".into(),
        ),
        "default owner-specific UDS registration delegates to unsupported legacy path"
    );
    assert!(
        !registry.register_uds_tool_for_owner_with_stable_id(
            std::sync::Arc::new(CovNoopTool),
            "uds:client:cov".into(),
            Some("com.example.cov-noop.v1".into()),
        ),
        "default stable-id UDS registration delegates to unsupported legacy path"
    );
    registry.set_session_key("covered");
    registry.register_runtime_tool(std::sync::Arc::new(CovNoopTool));
    registry.unregister_runtime_tool("cov_noop");
    assert!(registry.execute("missing", "{}").await.is_err());
}

#[test]
fn default_catalog_and_extension_trait_methods_are_exercised() {
    let mut registry = CovEmptyRegistry;
    // Default catalog descriptors fall back to Runtime/enabled metadata.
    assert!(registry.descriptors().is_empty());
    assert!(!registry.register_uds_tool(std::sync::Arc::new(CovNoopTool)));
    assert!(!registry.enable_tool("cov_noop"));
    assert!(!registry.disable_tool("cov_noop"));
}

struct CovOneDefRegistry {
    defs: Vec<ToolDefinition>,
}

impl ToolCatalog for CovOneDefRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &self.defs
    }
}

impl ToolExecutor for CovOneDefRegistry {
    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("missing".into())) })
    }
}

impl RuntimeToolLifecycleRegistry for CovOneDefRegistry {}
impl SessionAwareTools for CovOneDefRegistry {}
impl ToolPolicyMutator for CovOneDefRegistry {}
impl ToolRegistry for CovOneDefRegistry {}

#[test]
fn default_descriptors_map_definitions_to_runtime_source() {
    let registry = CovOneDefRegistry {
        defs: vec![ToolDefinition {
            name: "one".into(),
            description: "d".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }],
    };
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].name(), "one");
    assert_eq!(
        descriptors[0].source,
        crate::domain::tool_descriptor::ToolSource::Runtime
    );
    assert!(descriptors[0].availability.is_enabled());
}
