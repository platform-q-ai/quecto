// Native extension: wraps a compiled-in Tool as an Extension.
//
// Unlike ScriptTool (subprocess execution), native extensions are pure Rust
// implementations registered conditionally based on config. They have zero
// overhead when disabled and are not removed during reload_scripts().

use std::sync::Arc;

use crate::domain::extension::Extension;
use crate::domain::tool::Tool;

/// A compiled-in extension that wraps a `Tool` implementation.
///
/// Native extensions are registered conditionally at startup based on config
/// (e.g., `tools.web.brave.enabled`). They differ from script extensions in
/// that they:
/// - Execute in-process (no subprocess, no external runtime)
/// - Return `is_script() -> false` (not removed by `reload_scripts()`)
/// - Share the process's `reqwest::Client` and other resources
pub struct NativeExtension {
    name: String,
    description: String,
    tool: Arc<dyn Tool>,
    system_prompt: Option<String>,
}

impl std::fmt::Debug for NativeExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeExtension")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

impl NativeExtension {
    /// Create a new native extension wrapping a tool.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tool,
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
        vec![self.tool.clone()]
    }

    fn system_prompt_snippet(&self) -> Option<String> {
        self.system_prompt.clone()
    }

    fn is_script(&self) -> bool {
        false
    }
}

/// Build native extensions from config.
///
/// Currently supports:
/// - `web_search` — registered when `tools.web.brave.enabled` or
///   `tools.web.duckduckgo.enabled` is true in config.
///
/// Returns a list of extensions to register. Caller is responsible for
/// registering them via `ExtensionRegistry::register()` and/or
/// `ToolRegistryImpl::register_extension()`.
pub fn build_native_extensions(
    config: &crate::infrastructure::config::Config,
    http_client: &reqwest::Client,
) -> Vec<Arc<dyn Extension>> {
    let mut extensions: Vec<Arc<dyn Extension>> = Vec::new();

    // Web search: Brave or DuckDuckGo
    if config.tools.web.brave.enabled || config.tools.web.duckduckgo.enabled {
        let api_key =
            if config.tools.web.brave.enabled && !config.tools.web.brave.api_key.is_empty() {
                Some(config.tools.web.brave.api_key.clone())
            } else {
                None
            };

        let tool = Arc::new(
            crate::infrastructure::tools::web_search::WebSearchTool::with_client(
                api_key,
                http_client.clone(),
            ),
        );

        extensions.push(Arc::new(NativeExtension::new(
            "web_search",
            "Search the web using Brave Search or DuckDuckGo",
            tool,
        )));
    }

    extensions
}

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
    fn test_native_extension_is_not_script() {
        let ext = NativeExtension::new(
            "web_search",
            "Search the web",
            dummy_tool("web_search", "Search"),
        );
        assert!(!ext.is_script());
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
    fn test_native_extension_not_removed_by_reload_scripts() {
        let mut reg = crate::infrastructure::extensions::registry::ExtensionRegistry::new();
        let ext = Arc::new(NativeExtension::new(
            "web_search",
            "Search the web",
            dummy_tool("web_search", "Search"),
        ));
        reg.register(ext);
        assert_eq!(reg.all_tools().len(), 1);

        // reload_scripts only removes extensions where is_script() == true
        reg.reload_scripts();
        assert_eq!(
            reg.all_tools().len(),
            1,
            "native extension should survive reload"
        );
    }

    #[test]
    fn test_build_native_extensions_brave_enabled() {
        let config = config_with_web(true, "test-key", false);
        let client = reqwest::Client::new();
        let exts = build_native_extensions(&config, &client);
        assert_eq!(exts.len(), 1);
        assert_eq!(exts[0].name(), "web_search");
    }

    #[test]
    fn test_build_native_extensions_ddg_enabled() {
        let config = config_with_web(false, "", true);
        let client = reqwest::Client::new();
        let exts = build_native_extensions(&config, &client);
        assert_eq!(exts.len(), 1);
        assert_eq!(exts[0].name(), "web_search");
    }

    #[test]
    fn test_build_native_extensions_both_disabled() {
        let config = config_with_web(false, "", false);
        let client = reqwest::Client::new();
        let exts = build_native_extensions(&config, &client);
        assert!(exts.is_empty());
    }

    #[test]
    fn test_build_native_extensions_brave_enabled_no_key_falls_back() {
        // Brave enabled but no API key — should still register (DDG fallback)
        let config = config_with_web(true, "", false);
        let client = reqwest::Client::new();
        let exts = build_native_extensions(&config, &client);
        assert_eq!(exts.len(), 1);
    }

    fn config_with_web(
        brave_enabled: bool,
        brave_key: &str,
        ddg_enabled: bool,
    ) -> crate::infrastructure::config::Config {
        let mut config = crate::infrastructure::config::Config::default();
        config.tools.web.brave.enabled = brave_enabled;
        config.tools.web.brave.api_key = brave_key.to_string();
        config.tools.web.duckduckgo.enabled = ddg_enabled;
        config
    }
}
