// Native extension: wraps a compiled-in Tool as an Extension.
//
// Native extensions are pure Rust implementations registered conditionally
// based on config. They have zero overhead when disabled.

use std::sync::Arc;

use crate::domain::extension::Extension;
use crate::domain::tool::Tool;

/// A compiled-in extension that wraps a `Tool` implementation.
///
/// Native extensions are registered conditionally at startup based on config
/// (e.g., `tools.web.brave.enabled`). They:
/// - Execute in-process (no subprocess, no external runtime)
/// - Share the process's `reqwest::Client` and other resources
pub struct NativeExtension {
    name: String,
    description: String,
    tools: Vec<Arc<dyn Tool>>,
    system_prompt: Option<String>,
}

impl std::fmt::Debug for NativeExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeExtension")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

impl NativeExtension {
    /// Create a new native extension wrapping a single tool.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tools: vec![tool],
            system_prompt: None,
        }
    }

    /// Create a native extension wrapping multiple tools.
    pub fn with_tools(
        name: impl Into<String>,
        description: impl Into<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tools,
            system_prompt: None,
        }
    }

    /// Set an optional system prompt snippet.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }
}

impl Extension for NativeExtension {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    fn system_prompt_snippet(&self) -> Option<String> {
        self.system_prompt.clone()
    }
}

/// Build native extensions from web tool config.
///
/// Builds a single `"web"` extension containing whichever web tools are
/// enabled in config:
/// - `web_search` — registered when `brave.enabled` or `duckduckgo.enabled`
/// - `web_fetch` — registered when `fetch.enabled`
///
/// Returns a list of extensions to register. Caller is responsible for
/// registering them via `ExtensionRegistry::register()` and/or
/// `ToolRegistryImpl::register_extension()`.
pub fn build_native_extensions(
    web_config: &crate::infrastructure::config::WebToolConfig,
    http_client: &reqwest::Client,
) -> Vec<Arc<dyn Extension>> {
    let mut web_tools: Vec<Arc<dyn Tool>> = Vec::new();

    // Web search: Brave or DuckDuckGo
    if web_config.brave.enabled || web_config.duckduckgo.enabled {
        let api_key = if web_config.brave.enabled && !web_config.brave.api_key.is_empty() {
            Some(web_config.brave.api_key.clone())
        } else {
            None
        };
        web_tools.push(Arc::new(
            crate::infrastructure::tools::web_search::WebSearchTool::with_client(
                api_key,
                http_client.clone(),
            ),
        ));
    }

    // Web fetch
    if web_config.fetch.enabled {
        web_tools.push(Arc::new(
            crate::infrastructure::tools::web_fetch::WebFetchTool::with_client(
                http_client.clone(),
                web_config.fetch.max_response_kb,
            ),
        ));
    }

    let mut extensions: Vec<Arc<dyn Extension>> = Vec::new();
    if !web_tools.is_empty() {
        extensions.push(Arc::new(NativeExtension::with_tools(
            "web",
            "Web search and fetch",
            web_tools,
        )));
    }

    extensions
}

#[cfg(test)]
#[path = "native_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::error::DomainError;
    use crate::domain::tool::{ToolDefinition, ToolResult};
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

    #[test]
    fn test_native_extension_name() {
        let ext = NativeExtension::new(
            "web_search",
            "Search the web",
            dummy_tool("web_search", "Search"),
        );
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
        let ext =
            NativeExtension::with_tools("web", "Web tools", vec![]).with_system_prompt("prompt");
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
}

#[cfg(test)]
mod native_mock_surface_tests {
    use crate::domain::error::DomainError;
    use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
    use std::future::Future;
    use std::pin::Pin;

    #[derive(Debug)]
    struct SurfaceTool;

    impl Tool for SurfaceTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "surface".into(),
                description: "surface mock".into(),
                parameters_schema: r#"{"type":"object"}"#.into(),
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

    #[tokio::test]
    async fn tool_trait_surface_defaults_are_exercised() {
        let tool = SurfaceTool;
        assert_eq!(format!("{tool:?}"), "SurfaceTool");
        tool.set_session_key("session-key".into());

        let ToolDefinition {
            name, description, ..
        } = tool.definition();
        assert_eq!(name, "surface");
        assert_eq!(description, "surface mock");
        let result = tool.execute("{}").await.unwrap();
        assert_eq!(result.content, "ok");
        assert!(!result.is_error);
    }
}
