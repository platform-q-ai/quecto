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
    fn definition(&self) -> crate::domain::tool::ToolDefinition {
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

/// A tool that names every result a snapshot (#2342).
#[derive(Debug)]
struct Snapshotting(&'static str);

impl Tool for Snapshotting {
    fn snapshot_key(&self, _arguments: &str, _content: &str) -> Option<&'static str> {
        Some("state")
    }
    fn definition(&self) -> crate::domain::tool::ToolDefinition {
        Named(self.0).definition()
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("unused".into())) })
    }
}

/// #2342: only a bundled tool's snapshot key counts: a runtime or UDS tool
/// claiming one would let it collapse other results of that key.
#[test]
fn only_a_bundled_tool_names_a_snapshot() {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_runtime_tool(Arc::new(Snapshotting("extension_state"))));
    assert!(registry.register_uds_tool(Arc::new(Snapshotting("uds_state"))));
    assert!(registry.register(Arc::new(Named("bundled_plain"))));
    assert!(registry.register(Arc::new(Snapshotting("bundled_state"))));
    assert_eq!(registry.snapshot_key("extension_state", "{}", "x"), None);
    assert_eq!(registry.snapshot_key("uds_state", "{}", "x"), None);
    assert_eq!(registry.snapshot_key("bundled_plain", "{}", "x"), None);
    assert_eq!(registry.snapshot_key("not-a-tool", "{}", "x"), None);
    assert_eq!(
        registry.snapshot_key("bundled_state", "{}", "x"),
        Some("state")
    );
}

/// #2342: the bundled `swarm` tool is registered with its snapshot key.
#[test]
fn the_registry_asks_the_swarm_tool_for_its_snapshot_key() {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register(Arc::new(
        crate::infrastructure::tools::swarm::SwarmTool::new()
    )));
    let summary = r#"{"run":{},"members":[],"tasks":[],"event_cursor":7}"#;
    assert_eq!(
        registry.snapshot_key("swarm", r#"{"op":"summary"}"#, summary),
        Some(crate::infrastructure::tools::swarm::SUMMARY_SNAPSHOT)
    );
}
