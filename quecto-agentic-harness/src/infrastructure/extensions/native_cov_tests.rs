use super::*;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug)]
struct TinyTool;

impl Tool for TinyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tiny".into(),
            description: "Tiny".into(),
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

#[test]
fn debug_and_extension_methods_do_not_dump_tool_values() {
    let ext = NativeExtension::new("native", "Native desc", Arc::new(TinyTool))
        .with_system_prompt("Use native");

    assert_eq!(ext.name(), "native");
    assert_eq!(ext.description(), "Native desc");
    assert_eq!(ext.system_prompt_snippet().as_deref(), Some("Use native"));
    assert_eq!(ext.tools().len(), 1);
    let debug = format!("{ext:?}");
    assert!(debug.contains("native"));
    assert!(debug.contains("tool_count: 1"));
    assert!(!debug.contains("TinyTool"));
}

#[tokio::test]
async fn build_native_extensions_combines_search_and_fetch_in_one_web_extension() {
    let mut web = crate::infrastructure::config::WebToolConfig::default();
    web.brave.enabled = true;
    web.brave.api_key = "key".into();
    web.duckduckgo.enabled = true;
    web.fetch.enabled = true;
    web.fetch.max_response_kb = 1;
    let client = reqwest::Client::new();

    let tiny = TinyTool;
    tiny.set_session_key("native-cov".into());
    assert_eq!(tiny.definition().name.as_ref(), "tiny");
    assert_eq!(tiny.execute("{}").await.unwrap().content, "ok");

    let exts = build_native_extensions(&web, &client);

    assert_eq!(exts.len(), 1);
    assert_eq!(exts[0].name(), "web");
    let mut names: Vec<_> = exts[0]
        .tools()
        .iter()
        .map(|tool| tool.definition().name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["web_fetch", "web_search"]);
}
