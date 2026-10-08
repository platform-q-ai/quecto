//! #2169: which calls the registry lets run at once — bundled tools that
//! change nothing, and nothing else.
use super::*;
use crate::application::tools::ports::ToolExecutor;

#[derive(Debug)]
struct Named(&'static str);

impl Tool for Named {
    fn definition(&self) -> crate::domain::tool_policy::value_objects::tool::ToolDefinition {
        crate::domain::tool_policy::value_objects::tool::ToolDefinition {
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
        assert!(registry.overlaps_safely(name, "{}"), "{name} may overlap");
    }
    // Registered only in some compositions: when present, they may overlap.
    for name in ["web_fetch", "docs"] {
        if registry.get(name).is_some() {
            assert!(registry.overlaps_safely(name, "{}"), "{name} may overlap");
        }
    }
    // web_search is paced by its provider's rate limit (#2175 review).
    for name in ["bash", "write", "edit", "web_search", "recall"] {
        assert!(
            !registry.overlaps_safely(name, "{}"),
            "{name} may not overlap"
        );
    }
    assert!(!registry.overlaps_safely("not-a-tool", "{}"));
}

/// A tool that says its calls may overlap.
#[derive(Debug)]
struct Overlapping(&'static str);

impl Tool for Overlapping {
    fn overlaps_safely(&self, _arguments: &str) -> bool {
        true
    }
    fn definition(&self) -> crate::domain::tool_policy::value_objects::tool::ToolDefinition {
        Named(self.0).definition()
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("unused".into())) })
    }
}

/// Only a bundled tool that says so: an extension claiming it, or a
/// bundled tool that does not, runs one at a time.
#[test]
fn only_a_bundled_tool_that_says_so_overlaps() {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_runtime_tool(Arc::new(Overlapping("extension_read"))));
    assert!(registry.register_uds_tool(Arc::new(Overlapping("uds_read"))));
    assert!(registry.register(Arc::new(Named("bundled_quiet"))));
    assert!(registry.register(Arc::new(Overlapping("bundled_read"))));
    assert!(!registry.overlaps_safely("extension_read", "{}"));
    assert!(!registry.overlaps_safely("uds_read", "{}"));
    assert!(!registry.overlaps_safely("bundled_quiet", "{}"));
    assert!(registry.overlaps_safely("bundled_read", "{}"));
}

/// #2175 review: a ranked grep paces a rate-limited judge, so it runs one
/// at a time; a plain one may overlap.
#[test]
fn a_ranked_grep_does_not_overlap() {
    let (registry, _tmp) = super::super::tests::test_registry();
    assert!(registry.overlaps_safely("grep", r#"{"pattern":"x"}"#));
    assert!(!registry.overlaps_safely("grep", r#"{"pattern":"x","rank_by":"y"}"#));
}

/// A tool that records the collapses it is told of (#2348 review M1).
#[derive(Debug, Default)]
struct Forgetting(std::sync::Mutex<Vec<String>>);

impl Tool for Forgetting {
    fn result_collapsed(&self, arguments: &str) {
        self.0.lock().unwrap().push(arguments.to_string());
    }
    fn definition(&self) -> crate::domain::tool_policy::value_objects::tool::ToolDefinition {
        Named("read").definition()
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("unused".into())) })
    }
}

/// #2348 review M1: the registry tells the bundled tool its result was
/// collapsed; an unknown name is told nothing.
#[test]
fn the_registry_tells_a_bundled_tool_its_result_was_collapsed() {
    let mut registry = ToolRegistryImpl::new();
    let tool = Arc::new(Forgetting::default());
    assert!(registry.register(tool.clone()));
    registry.result_collapsed("read", r#"{"path":"a.rs"}"#);
    registry.result_collapsed("not-a-tool", "{}");
    assert_eq!(
        *tool.0.lock().unwrap(),
        vec![r#"{"path":"a.rs"}"#.to_string()]
    );
}
