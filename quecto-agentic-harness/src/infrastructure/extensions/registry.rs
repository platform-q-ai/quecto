//! Extension registry: manages extensions.
//!
//! Holds registered extensions and provides aggregate access to their
//! tools and system prompt snippets.

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::extension::Extension;
use crate::domain::tool::Tool;

/// Registry of all extensions (native and UDS-registered).
pub struct ExtensionRegistry {
    extensions: Vec<Arc<dyn Extension>>,
}

impl std::fmt::Debug for ExtensionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionRegistry")
            .field("extension_count", &self.extensions.len())
            .finish()
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    /// Register an extension.
    pub fn register(&mut self, ext: Arc<dyn Extension>) {
        self.extensions.push(ext);
    }

    /// All tools across all extensions, deduplicated by name (last wins).
    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut map: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        for ext in &self.extensions {
            for tool in ext.tools() {
                let name = tool.definition().name.to_string();
                map.insert(name, tool);
            }
        }
        map.into_values().collect()
    }

    /// All non-empty system prompt snippets, concatenated with double newlines.
    pub fn system_prompt_snippets(&self) -> String {
        self.extensions
            .iter()
            .filter_map(|ext| ext.system_prompt_snippet())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Return the number of registered extensions.
    pub fn extension_count(&self) -> usize {
        self.extensions.len()
    }
}

#[cfg(test)]
#[path = "registry_cov_tests.rs"]
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

    struct TestExt {
        name: String,
        tools: Vec<Arc<dyn Tool>>,
        snippet: Option<String>,
    }

    impl Extension for TestExt {
        fn name(&self) -> &str {
            &self.name
        }
        fn tools(&self) -> Vec<Arc<dyn Tool>> {
            self.tools.clone()
        }
        fn system_prompt_snippet(&self) -> Option<String> {
            self.snippet.clone()
        }
    }

    #[test]
    fn test_empty_registry() {
        let reg = ExtensionRegistry::new();
        assert!(reg.all_tools().is_empty());
        assert!(reg.system_prompt_snippets().is_empty());
        assert_eq!(reg.extension_count(), 0);
    }

    #[tokio::test]
    async fn test_register_and_get_tools() {
        let mut reg = ExtensionRegistry::new();
        let ext = Arc::new(TestExt {
            name: "test".into(),
            tools: vec![Arc::new(DummyTool {
                name: "mytool".into(),
                desc: "desc".into(),
            })],
            snippet: None,
        });
        assert_eq!(ext.name(), "test");
        assert_eq!(ext.description(), "");
        reg.register(ext);
        let tools = reg.all_tools();
        assert_eq!(tools.len(), 1);
        tools[0].set_session_key("registry-test".into());
        assert_eq!(tools[0].execute("{}").await.unwrap().content, "ok");
    }

    #[test]
    fn test_dedup_last_wins() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Arc::new(TestExt {
            name: "ext1".into(),
            tools: vec![Arc::new(DummyTool {
                name: "shared".into(),
                desc: "first".into(),
            })],
            snippet: None,
        }));
        reg.register(Arc::new(TestExt {
            name: "ext2".into(),
            tools: vec![Arc::new(DummyTool {
                name: "shared".into(),
                desc: "second".into(),
            })],
            snippet: None,
        }));
        let tools = reg.all_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].definition().description.as_ref(), "second");
    }

    #[test]
    fn test_prompt_snippets() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Arc::new(TestExt {
            name: "a".into(),
            tools: vec![],
            snippet: Some("Snippet A".into()),
        }));
        reg.register(Arc::new(TestExt {
            name: "b".into(),
            tools: vec![],
            snippet: Some("Snippet B".into()),
        }));
        let snippets = reg.system_prompt_snippets();
        assert!(snippets.contains("Snippet A"));
        assert!(snippets.contains("Snippet B"));
    }

    #[test]
    fn test_prompt_snippets_skip_empty() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Arc::new(TestExt {
            name: "a".into(),
            tools: vec![],
            snippet: Some("".into()),
        }));
        reg.register(Arc::new(TestExt {
            name: "b".into(),
            tools: vec![],
            snippet: None,
        }));
        assert!(reg.system_prompt_snippets().is_empty());
    }
}

#[cfg(test)]
mod registry_mock_surface_tests {
    use crate::domain::error::DomainError;
    use crate::domain::extension::Extension;
    use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

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

    struct SurfaceExt {
        tools: Vec<Arc<dyn Tool>>,
    }

    impl Extension for SurfaceExt {
        fn name(&self) -> &str {
            "surface-ext"
        }

        fn tools(&self) -> Vec<Arc<dyn Tool>> {
            self.tools.clone()
        }
    }

    #[tokio::test]
    async fn extension_and_tool_trait_surface_defaults_are_exercised() {
        let tool = Arc::new(SurfaceTool);
        tool.set_session_key("session-key".into());
        assert_eq!(tool.definition().name, "surface");
        assert_eq!(tool.execute("{}").await.unwrap().content, "ok");

        let ext = SurfaceExt { tools: vec![tool] };
        assert_eq!(ext.name(), "surface-ext");
        assert_eq!(ext.description(), "");
        assert_eq!(ext.tools().len(), 1);
        assert!(ext.system_prompt_snippet().is_none());
    }
}
