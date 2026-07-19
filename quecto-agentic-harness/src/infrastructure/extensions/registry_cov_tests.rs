use super::*;
use crate::domain::error::DomainError;
use crate::domain::extension::Extension;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
struct NamedTool(&'static str);

impl Tool for NamedTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.0.into(),
            description: format!("tool {}", self.0).into(),
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

struct Ext {
    name: &'static str,
    tools: Vec<Arc<dyn Tool>>,
    snippet: Option<&'static str>,
}

impl Extension for Ext {
    fn name(&self) -> &str {
        self.name
    }
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }
    fn system_prompt_snippet(&self) -> Option<String> {
        self.snippet.map(str::to_owned)
    }
}

#[test]
fn default_debug_and_prompt_filtering_are_observable() {
    let mut reg = ExtensionRegistry::default();
    assert_eq!(reg.extension_count(), 0);
    assert!(format!("{reg:?}").contains("extension_count: 0"));

    reg.register(Arc::new(Ext {
        name: "empty",
        tools: vec![],
        snippet: Some(""),
    }));
    reg.register(Arc::new(Ext {
        name: "full",
        tools: vec![Arc::new(NamedTool("alpha"))],
        snippet: Some("Use alpha"),
    }));

    assert_eq!(reg.extension_count(), 2);
    assert_eq!(reg.system_prompt_snippets(), "Use alpha");
    let tools = reg.all_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].definition().name.as_ref(), "alpha");
    assert!(format!("{reg:?}").contains("extension_count: 2"));
}
