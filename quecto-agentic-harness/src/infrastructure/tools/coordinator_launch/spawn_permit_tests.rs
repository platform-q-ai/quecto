//! S4c: the spawn permit follows the registry's policy over `spawn`,
//! as a parent's own call is judged.
use std::sync::Arc;

use super::SpawnPermit;
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::tools::registry::ToolRegistryImpl;

/// A tool by `name` that does nothing.
struct Named(&'static str);

impl Tool for Named {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.0.into(),
            description: "a test tool".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn execute(
        &self,
        _: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<ToolResult, DomainError>> + Send + '_>,
    > {
        Box::pin(async { Err(DomainError::Tool("never run".into())) })
    }
}

#[test]
fn the_permit_follows_the_registrys_policy_over_spawn() {
    let permit = SpawnPermit::default();
    assert!(!permit.allowed(), "closed until a registry says otherwise");
    let mut registry = ToolRegistryImpl::new();
    registry.watch_spawn_permit(permit.clone());
    assert!(!permit.allowed(), "no spawn tool, no spawn");
    registry.register(Arc::new(Named("spawn")));
    assert!(permit.allowed(), "a registered, enabled spawn");
    registry.disable_tool("spawn");
    assert!(!permit.allowed(), "disabled at runtime");
    registry.enable_tool("spawn");
    assert!(permit.allowed(), "enabled again");
    registry.disable_tool_by_entrypoint_default("spawn");
    assert!(!permit.allowed(), "off by the entrypoint's default");
    let other = SpawnPermit::default();
    let mut unrelated = ToolRegistryImpl::new();
    unrelated.register(Arc::new(Named("read")));
    unrelated.watch_spawn_permit(other.clone());
    assert!(!other.allowed(), "another tool is no spawn");
}
