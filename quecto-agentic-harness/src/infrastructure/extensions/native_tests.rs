//! Unit tests for the parent module (moved out of the production file so
//! test-only mocks do not count toward the production coverage denominator).

use super::*;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::extensions::native::OfficialToolDeps;
use std::future::Future;
use std::pin::Pin;

struct DummyTool {
    name: String,
    desc: String,
}

impl std::fmt::Debug for DummyTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DummyTool")
            .field("name", &self.name)
            .finish()
    }
}

impl Tool for DummyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone().into(),
            description: self.desc.clone().into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        _: &str,
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

fn dummy_tool(name: &str, desc: &str) -> Arc<dyn Tool> {
    Arc::new(DummyTool {
        name: name.into(),
        desc: desc.into(),
    })
}

#[tokio::test]
async fn test_native_extension_name() {
    let tool = dummy_tool("web_search", "Search");
    tool.set_session_key("native-test".into());
    assert_eq!(tool.execute("{}").await.unwrap().content, "ok");
    let debug = format!(
        "{:?}",
        DummyTool {
            name: "dbg".into(),
            desc: "Debug desc".into()
        }
    );
    assert!(debug.contains("DummyTool"));
    assert!(debug.contains("dbg"));

    let ext = NativeExtension::new("web_search", "Search the web", tool);
    assert_eq!(ext.name(), "web_search");
}

#[test]
fn test_native_extension_description() {
    let ext = NativeExtension::new(
        "web_search",
        "Search the web",
        dummy_tool("web_search", "Search"),
    );
    assert_eq!(ext.description(), "Search the web");
}

#[test]
fn test_native_extension_provides_one_tool() {
    let ext = NativeExtension::new(
        "web_search",
        "Search the web",
        dummy_tool("web_search", "Search"),
    );
    assert_eq!(ext.tools().len(), 1);
    assert_eq!(ext.tools()[0].definition().name.as_ref(), "web_search");
}

#[test]
fn test_native_extension_no_system_prompt_by_default() {
    let ext = NativeExtension::new(
        "web_search",
        "Search the web",
        dummy_tool("web_search", "Search"),
    );
    assert!(ext.system_prompt_snippet().is_none());
}

#[test]
fn test_native_extension_with_system_prompt() {
    let ext = NativeExtension::new(
        "web_search",
        "Search the web",
        dummy_tool("web_search", "Search"),
    )
    .with_system_prompt("Use web_search to find information.");
    assert_eq!(
        ext.system_prompt_snippet().as_deref(),
        Some("Use web_search to find information.")
    );
}

#[test]
fn test_native_extension_with_tools() {
    let ext = NativeExtension::with_tools(
        "web",
        "Web tools",
        vec![
            dummy_tool("web_search", "Search"),
            dummy_tool("web_fetch", "Fetch"),
        ],
    );
    assert_eq!(ext.name(), "web");
    assert_eq!(ext.tools().len(), 2);
    let names: Vec<_> = ext
        .tools()
        .iter()
        .map(|t| t.definition().name.to_string())
        .collect();
    assert!(names.contains(&"web_search".to_string()));
    assert!(names.contains(&"web_fetch".to_string()));
}

#[test]
fn test_native_extension_with_tools_system_prompt() {
    let ext = NativeExtension::with_tools("web", "Web tools", vec![]).with_system_prompt("prompt");
    assert_eq!(ext.system_prompt_snippet().as_deref(), Some("prompt"));
}

#[test]
fn test_build_native_extensions_brave_enabled() {
    let web = web_config(true, "test-key", false, false);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "web");
    assert!(has_tool(&exts, "web_search"));
}

#[test]
fn test_build_native_extensions_ddg_enabled() {
    let web = web_config(false, "", true, false);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "web");
    assert!(has_tool(&exts, "web_search"));
}

#[test]
fn test_build_native_extensions_all_disabled() {
    let web = web_config(false, "", false, false);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert!(exts.is_empty());
}

#[test]
fn test_build_native_extensions_brave_enabled_no_key_falls_back() {
    let web = web_config(true, "", false, false);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert_eq!(exts.len(), 1);
}

#[test]
fn test_build_native_extensions_fetch_enabled() {
    let web = web_config(false, "", false, true);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "web");
    assert!(has_tool(&exts, "web_fetch"));
    assert!(!has_tool(&exts, "web_search"));
}

#[test]
fn test_build_native_extensions_search_and_fetch() {
    let web = web_config(true, "key", false, true);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "web");
    assert!(has_tool(&exts, "web_search"));
    assert!(has_tool(&exts, "web_fetch"));
}

#[test]
fn test_build_native_extensions_fetch_disabled() {
    let web = web_config(true, "key", false, false);
    let client = reqwest::Client::new();
    let exts = build_native_extensions(&web, &client);
    assert!(has_tool(&exts, "web_search"));
    assert!(!has_tool(&exts, "web_fetch"));
}

fn web_config(
    brave_enabled: bool,
    brave_key: &str,
    ddg_enabled: bool,
    fetch_enabled: bool,
) -> crate::infrastructure::config::WebToolConfig {
    let mut web = crate::infrastructure::config::WebToolConfig::default();
    web.brave.enabled = brave_enabled;
    web.brave.api_key = brave_key.to_string();
    web.duckduckgo.enabled = ddg_enabled;
    web.fetch.enabled = fetch_enabled;
    web
}

fn has_tool(exts: &[Arc<dyn Extension>], name: &str) -> bool {
    exts.iter()
        .flat_map(|e| e.tools())
        .any(|t| t.definition().name.as_ref() == name)
}

/// #1276 Phase 3 characterization: the official provider always supplies the
/// workspace/search/docs surface under the stable extension owner id.
#[test]
fn build_official_tool_extensions_lists_core_workspace_tools() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(
        Some(tmp.path().to_path_buf()),
        true,
    );
    let exts = build_official_tool_extensions(OfficialToolDeps {
        workspace: tmp.path().to_path_buf(),
        sandbox,
        exec_options: crate::infrastructure::tools::bash::ExecOptions::default(),
        docs_content_policy: crate::infrastructure::tools::docs::DocsContentPolicy::Parent,
        python_lab_config: crate::infrastructure::tools::python_lab::PythonLabConfig::default(),
    });
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "quecto:official-tools");
    for name in [
        "bash",
        "read",
        "write",
        "edit",
        "ls",
        "grep",
        "python_lab",
        "find",
        "docs",
    ] {
        assert!(has_tool(&exts, name), "missing official tool {name}");
    }
}

/// #1276 Phase 3: session-scoped provider supplies recall with the expected name.
#[test]
fn build_session_tool_extensions_supplies_recall() {
    use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
    use std::sync::Arc;

    let tmp = tempfile::TempDir::new().unwrap();
    let exts = build_session_tool_extensions(SessionToolDeps {
        spill_store: Arc::new(FileContextSpillStore::new(tmp.path().to_path_buf())),
        session_key: "cli:test".into(),
    });
    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "quecto:session-tools");
    assert!(has_tool(&exts, "recall"));
}

/// #1276 Phase 3: agent-control provider supplies spawn + agent_cmd and live handles.
#[test]
fn build_agent_control_tool_extensions_supplies_spawn_and_agent_cmd() {
    let tmp = tempfile::TempDir::new().unwrap();
    let built = build_agent_control_tool_extensions(AgentControlToolDeps {
        parent_config_path: None,
        base_dir: tmp.path().to_path_buf(),
        socket_dir: tmp.path().to_path_buf(),
        restrict_to_workspace: true,
        broadcast_tx: None,
        parent_session_name: Some("parent".into()),
        inherited_tool_policy: None,
    });
    assert_eq!(built.extensions.len(), 1);
    assert_eq!(built.extensions[0].name(), "quecto:agent-control");
    assert!(has_tool(&built.extensions, "spawn"));
    assert!(has_tool(&built.extensions, "agent_cmd"));
    // Handles must remain live for composition-root wiring (#926).
    let _ = built.subagent_registry;
    let _ = built.notification_tx;
    let _ = built.notification_rx;
}

/// #1276 Phase 3: workflow provider wraps an existing engine handle.
#[test]
fn build_workflow_tool_extension_supplies_workflow() {
    let engine = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    let ext = build_workflow_tool_extension(WorkflowToolDeps {
        engine,
        event_emitter: None,
    });
    assert_eq!(ext.name(), "quecto:workflow");
    assert!(has_tool(&[ext], "workflow"));
}

/// #1276 Phase 3: register_bundled_native_tools uses official (non-extension) path.
#[test]
fn register_bundled_native_tools_marks_official_not_extension_tracked() {
    use crate::domain::tool_descriptor::ToolSource;
    use crate::infrastructure::security::sandbox::Sandbox;
    use crate::infrastructure::tools::registry::ToolRegistryImpl;

    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), true);
    let mut registry = ToolRegistryImpl::new();
    register_bundled_native_tools(
        &mut registry,
        build_official_tool_extensions(OfficialToolDeps {
            workspace: tmp.path().to_path_buf(),
            sandbox,
            exec_options: crate::infrastructure::tools::bash::ExecOptions::default(),
            docs_content_policy: crate::infrastructure::tools::docs::DocsContentPolicy::Parent,
            python_lab_config: crate::infrastructure::tools::python_lab::PythonLabConfig::default(),
        }),
    );
    assert!(registry.get("bash").is_some());
    let d = registry.descriptor("bash").unwrap();
    assert!(matches!(d.source, ToolSource::BundledNative));
    assert_eq!(d.owner.as_ref(), "quecto:official-tools");
    let bash_entry = registry
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "bash")
        .expect("bash catalogue entry");
    assert_eq!(bash_entry.provider_id.as_ref(), "quecto:official-tools");
    assert!(!registry.runtime_tool_names().iter().any(|n| n == "bash"));
}
