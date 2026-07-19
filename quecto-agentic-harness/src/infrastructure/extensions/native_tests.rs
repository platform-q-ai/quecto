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
