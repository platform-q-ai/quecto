//! Contract tests for the `ToolCatalog` role port.
//!
//! Contract:
//! - `definitions()` lists every registered tool visible to the model.
//! - `tool_count()` matches `definitions().len()` for the default adapter.

use quecto::domain::tool::{Tool, ToolCatalog, ToolDefinition, ToolResult};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct Echo {
    name: Cow<'static, str>,
}

impl Tool for Echo {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: Cow::Borrowed("echo"),
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
        let content = arguments.to_string();
        Box::pin(async move {
            Ok(ToolResult {
                content,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

fn new_catalog_with(tools: Vec<&'static str>) -> Arc<dyn ToolCatalog> {
    let mut reg = ToolRegistryImpl::new();
    for name in tools {
        reg.register(Arc::new(Echo {
            name: Cow::Borrowed(name),
        }));
    }
    Arc::new(reg)
}

#[test]
fn empty_catalog_has_no_definitions() {
    let catalog = new_catalog_with(vec![]);
    assert_eq!(catalog.definitions().len(), 0);
    assert_eq!(catalog.tool_count(), 0);
}

#[test]
fn definitions_cover_every_registered_tool() {
    let catalog = new_catalog_with(vec!["alpha", "beta", "gamma"]);
    let names: Vec<_> = catalog
        .definitions()
        .iter()
        .map(|d| d.name.as_ref())
        .collect();
    assert_eq!(catalog.tool_count(), 3);
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}
