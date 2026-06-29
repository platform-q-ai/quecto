use super::*;

// Tool Empty Arguments Steps
// ===========================================================================

/// Execute a tool with a raw argument string (not a Gherkin table).
/// This lets us test with truly empty strings, whitespace-only, or
/// valid/invalid JSON to exercise argument normalisation paths.
#[when(expr = "the agent executes tool {string} with raw arguments {string}")]
fn when_agent_executes_tool_raw_args(world: &mut QuectoWorld, tool_name: String, raw_args: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, &raw_args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

/// Assert that a tool definition's description contains the expected substring.
#[then(expr = "the tool definition for {string} should contain {string}")]
fn then_tool_definition_contains(world: &mut QuectoWorld, tool_name: String, expected: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let defs = registry.definitions();
    let def = defs
        .iter()
        .find(|d| d.name == tool_name)
        .unwrap_or_else(|| panic!("tool definition for '{}' not found", tool_name));
    assert!(
        def.description.contains(&expected),
        "expected tool '{}' description to contain '{}', got: {}",
        tool_name,
        expected,
        def.description
    );
}
