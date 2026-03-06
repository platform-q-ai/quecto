//! Extension registry: discovers, loads, and manages extensions.
//!
//! Holds registered extensions and provides aggregate access to their
//! tools and system prompt snippets.  Supports hot-reload of script
//! extensions via `reload_scripts()`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::extension::Extension;
use crate::domain::tool::Tool;

use super::script::discover_script_extensions;

/// Registry of all extensions (compiled-in and script-based).
pub struct ExtensionRegistry {
    extensions: Vec<Arc<dyn Extension>>,
    watch_dirs: Vec<PathBuf>,
}

impl std::fmt::Debug for ExtensionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionRegistry")
            .field("extension_count", &self.extensions.len())
            .field("watch_dirs", &self.watch_dirs)
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
            watch_dirs: Vec::new(),
        }
    }

    /// Set directories to watch for script extensions.
    pub fn set_watch_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.watch_dirs = dirs;
    }

    /// Get the configured watch directories.
    pub fn watch_dirs(&self) -> &[PathBuf] {
        &self.watch_dirs
    }

    /// Discover script extensions from configured watch directories.
    pub fn discover(dirs: &[PathBuf]) -> Self {
        let mut reg = Self::new();
        reg.watch_dirs = dirs.to_vec();
        for dir in dirs {
            for ext in discover_script_extensions(dir) {
                reg.extensions.push(ext);
            }
        }
        reg
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

    /// Re-scan watched directories, replacing script extensions.
    /// Non-script extensions (builtins) are retained.
    pub fn reload_scripts(&mut self) {
        // Remove script extensions
        self.extensions
            .retain(|ext| !is_script_extension(ext.as_ref()));

        // Re-discover from watched directories
        for dir in &self.watch_dirs {
            for ext in discover_script_extensions(dir) {
                self.extensions.push(ext);
            }
        }
    }

    /// Return the number of registered extensions.
    pub fn extension_count(&self) -> usize {
        self.extensions.len()
    }
}

/// Check if an extension is a script extension.
///
/// We use the extension name prefix convention rather than downcasting
/// since `dyn Extension` is not `dyn Any`.  Script extensions discovered
/// from disk are tagged by the `ScriptExtension` wrapper which implements
/// a marker method.
fn is_script_extension(ext: &dyn Extension) -> bool {
    ext.is_script()
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

    #[test]
    fn test_register_and_get_tools() {
        let mut reg = ExtensionRegistry::new();
        reg.register(Arc::new(TestExt {
            name: "test".into(),
            tools: vec![Arc::new(DummyTool {
                name: "mytool".into(),
                desc: "desc".into(),
            })],
            snippet: None,
        }));
        assert_eq!(reg.all_tools().len(), 1);
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
