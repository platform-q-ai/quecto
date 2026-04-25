use cucumber::{World, given, then, when};
use quecto_mcp::{McpTool, build_registration, filter_tools, mcp_name_to_quecto_name};

#[derive(Debug, Default, World)]
struct McpWorld {
    tool: Option<McpTool>,
    tools: Vec<McpTool>,
    mapped_name: Option<String>,
    filtered: Vec<McpTool>,
    registration: Option<quecto_mcp::QuectoToolRegistration>,
}

#[given(expr = "an MCP tool named {string}")]
fn given_mcp_tool_named(world: &mut McpWorld, name: String) {
    world.tool = Some(McpTool {
        name,
        description: String::new(),
        input_schema: serde_json::json!({"type": "object"}),
    });
}

#[given("discovered MCP tools:")]
fn given_discovered_tools(world: &mut McpWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("table");
    world.tools = table
        .rows
        .iter()
        .skip(1)
        .map(|row| McpTool {
            name: row[0].clone(),
            description: String::new(),
            input_schema: serde_json::json!({"type": "object"}),
        })
        .collect();
}

#[given(expr = "the MCP tool description is {string}")]
fn given_description(world: &mut McpWorld, description: String) {
    world.tool.as_mut().expect("tool").description = description;
}

#[given(expr = "the MCP tool input schema is {string}")]
fn given_schema(world: &mut McpWorld, schema: String) {
    let schema = schema.replace("\\\"", "\"");
    world.tool.as_mut().expect("tool").input_schema = serde_json::from_str(&schema).unwrap();
}

#[when("I map the MCP tool name for Quecto")]
fn when_map_name(world: &mut McpWorld) {
    let tool = world.tool.as_ref().expect("tool");
    world.mapped_name = Some(mcp_name_to_quecto_name(&tool.name).unwrap());
}

#[when(expr = "I filter tools with prefix {string}")]
fn when_filter_prefix(world: &mut McpWorld, prefix: String) {
    world.filtered = filter_tools(&world.tools, &[prefix], &[], &[]).unwrap();
}

#[when("I build a Quecto registration")]
fn when_build_registration(world: &mut McpWorld) {
    let tool = world.tool.as_ref().expect("tool");
    world.registration = Some(build_registration(tool).unwrap());
}

#[then(expr = "the Quecto tool name should be {string}")]
fn then_quecto_name(world: &mut McpWorld, expected: String) {
    let actual = world
        .mapped_name
        .as_ref()
        .cloned()
        .or_else(|| world.registration.as_ref().map(|r| r.name.clone()))
        .expect("name");
    assert_eq!(actual, expected);
}

#[then("the filtered MCP tool names should be:")]
fn then_filtered_names(world: &mut McpWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("table");
    let expected: Vec<String> = table
        .rows
        .iter()
        .skip(1)
        .map(|row| row[0].clone())
        .collect();
    let actual: Vec<String> = world
        .filtered
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(actual, expected);
}

#[then(expr = "the Quecto tool description should be {string}")]
fn then_description(world: &mut McpWorld, expected: String) {
    assert_eq!(
        world
            .registration
            .as_ref()
            .expect("registration")
            .description,
        expected
    );
}

#[then(expr = "the Quecto tool schema should be {string}")]
fn then_schema(world: &mut McpWorld, expected: String) {
    let expected = expected.replace("\\\"", "\"");
    assert_eq!(
        world
            .registration
            .as_ref()
            .expect("registration")
            .parameters_schema,
        expected
    );
}

#[tokio::main]
async fn main() {
    McpWorld::cucumber()
        .max_concurrent_scenarios(1)
        .filter_run("tests/features", |_feat, _rule, _sc| true)
        .await;
}
