//! Contract tests for the `ExtensionToolRegistry` role port.
//!
//! Contract:
//! - extension tools are named separately from core tools.
//! - unregister removes extension tools without allowing removal of core tools.
//! - extension tools cannot shadow core tools.

use quecto::domain::tool::{
    ExtensionToolRegistry, Tool, ToolCatalog, ToolDefinition, ToolExecutor, ToolResult,
};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct Echo {
    name: Cow<'static, str>,
    marker: Cow<'static, str>,
}

impl Echo {
    fn new(name: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            marker: Cow::Borrowed("echo"),
        }
    }

    fn with_marker(name: &'static str, marker: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            marker: Cow::Borrowed(marker),
        }
    }
}

impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: self.marker.clone(),
            parameters_schema: Cow::Borrowed(r#"{"type":"object"}"#),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ToolResult, quecto::domain::error::DomainError>> + Send + '_,
        >,
    > {
        let content = format!("{}:{arguments}", self.marker);
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

#[test]
fn extension_names_track_lifecycle_for_multiple_extensions_only() {
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Echo::new("core")));
    registry.register_extension(Arc::new(Echo::new("extension_a")));
    registry.register_extension(Arc::new(Echo::new("extension_b")));

    {
        let extension_registry: &mut dyn ExtensionToolRegistry = &mut registry;
        extension_registry.unregister_extension("extension_a");
    }

    let mut extension_names = registry.extension_names();
    extension_names.sort();
    assert_eq!(extension_names, vec!["extension_b"]);
    assert!(registry.names().contains(&"core".to_string()));
    assert!(!registry.names().contains(&"extension_a".to_string()));
    assert!(registry.names().contains(&"extension_b".to_string()));
}

#[test]
fn unregister_extension_removes_extension_without_removing_core_tools() {
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Echo::new("core")));
    registry.register_extension(Arc::new(Echo::new("extension")));

    {
        let extension_registry: &mut dyn ExtensionToolRegistry = &mut registry;
        extension_registry.unregister_extension("core");
        extension_registry.unregister_extension("extension");
    }

    assert!(registry.names().contains(&"core".to_string()));
    assert!(!registry.names().contains(&"extension".to_string()));
    assert!(registry.extension_names().is_empty());
}

#[tokio::test]
async fn registered_extension_tools_are_cataloged_executable_and_removed_on_unregister() {
    let mut registry = ToolRegistryImpl::new();
    registry.register_extension(Arc::new(Echo::with_marker("extension", "extension")));

    let catalog: &dyn ToolCatalog = &registry;
    assert_eq!(catalog.tool_count(), 1);
    assert_eq!(catalog.definitions()[0].name.as_ref(), "extension");

    let executor: &dyn ToolExecutor = &registry;
    assert_eq!(
        executor
            .execute("extension", "payload")
            .await
            .expect("registered extension must execute")
            .content,
        "extension:payload"
    );

    {
        let extension_registry: &mut dyn ExtensionToolRegistry = &mut registry;
        extension_registry.unregister_extension("extension");
    }

    assert_eq!(registry.definitions().len(), 0);
    assert!(registry.execute("extension", "payload").await.is_err());
}

#[test]
fn extension_tools_cannot_shadow_core_tools() {
    let mut registry = ToolRegistryImpl::new();
    registry.register(Arc::new(Echo::with_marker("same", "core")));

    {
        let extension_registry: &mut dyn ExtensionToolRegistry = &mut registry;
        extension_registry.register_extension(Arc::new(Echo::with_marker("same", "extension")));
    }

    assert!(registry.extension_names().is_empty());
    assert_eq!(registry.names(), vec!["same"]);
    assert_eq!(
        registry.definitions()[0].description.as_ref(),
        "core",
        "shadow rejection must leave the original core tool definition intact"
    );
}
