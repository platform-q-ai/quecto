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
#[path = "registry_tests.rs"]
mod tests;
