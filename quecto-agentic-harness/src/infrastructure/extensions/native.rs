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
#[path = "native_tests.rs"]
mod tests;
