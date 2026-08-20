//! Unit tests for the parent module (moved out of the production file so
//! test-only mocks do not count toward the production coverage denominator).

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
                delivery_metadata: None,
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
