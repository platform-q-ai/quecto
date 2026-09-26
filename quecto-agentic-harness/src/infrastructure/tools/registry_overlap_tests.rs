//! #2169: which calls the registry lets run at once — bundled tools that
//! change nothing, and nothing else.
use super::*;
use crate::application::tools::ports::ToolExecutor;

#[derive(Debug)]
struct Named(&'static str);

impl Tool for Named {
    fn definition(&self) -> crate::domain::tool::ToolDefinition {
        crate::domain::tool::ToolDefinition {
            name: self.0.into(),
            description: "named".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("unused".into())) })
    }
}

#[test]
fn bundled_tools_that_change_nothing_may_overlap() {
    let (registry, _tmp) = super::super::tests::test_registry();
    for name in ["read", "grep", "find", "ls"] {
        assert!(registry.get(name).is_some(), "{name} is registered");
        assert!(registry.overlaps_safely(name), "{name} may overlap");
    }
    for name in ["bash", "write", "edit"] {
        assert!(registry.get(name).is_some(), "{name} is registered");
        assert!(!registry.overlaps_safely(name), "{name} may not overlap");
    }
    assert!(!registry.overlaps_safely("not-a-tool"));
}

/// Only the bundled tool of that name: an extension reusing a read-only
/// name decides nothing.
#[test]
fn an_extension_tool_never_overlaps_whatever_its_name() {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_runtime_tool(Arc::new(Named("docs"))));
    assert!(registry.register_uds_tool(Arc::new(Named("recall"))));
    assert!(!registry.overlaps_safely("docs"));
    assert!(!registry.overlaps_safely("recall"));
    assert!(registry.register(Arc::new(Named("web_search"))));
    assert!(registry.overlaps_safely("web_search"));
}
