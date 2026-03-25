use super::tests::{MockProvider, MockRegistry, MockTool};
use super::*;
use crate::domain::agent::AgentLoop;
use std::sync::Arc;

#[test]
fn test_swap_registry_replaces_tool_registry() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let mut reg_a = MockRegistry::new();
    reg_a.register(Arc::new(MockTool::new("tool_a", "ok")));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(reg_a),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
    });
    assert_eq!(agent.info().tool_count, 1);
    assert_eq!(agent.tool_registry.definitions()[0].name.as_ref(), "tool_a");

    let mut reg_b = MockRegistry::new();
    reg_b.register(Arc::new(MockTool::new("tool_b", "ok")));
    reg_b.register(Arc::new(MockTool::new("tool_c", "ok")));
    agent.swap_registry(Box::new(reg_b));
    assert_eq!(agent.info().tool_count, 2);
    let names: Vec<&str> = agent
        .tool_registry
        .definitions()
        .iter()
        .map(|d| d.name.as_ref())
        .collect();
    assert!(names.contains(&"tool_b"));
    assert!(names.contains(&"tool_c"));
    assert!(!names.contains(&"tool_a"));
}

#[test]
fn test_swap_registry_info_reflects_new_count() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let reg = MockRegistry::new();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(reg),
        model: "m".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
    });
    assert_eq!(agent.info().tool_count, 0);

    let mut new_reg = MockRegistry::new();
    new_reg.register(Arc::new(MockTool::new("alpha", "ok")));
    new_reg.register(Arc::new(MockTool::new("beta", "ok")));
    new_reg.register(Arc::new(MockTool::new("gamma", "ok")));
    agent.swap_registry(Box::new(new_reg));
    assert_eq!(agent.info().tool_count, 3);
}
