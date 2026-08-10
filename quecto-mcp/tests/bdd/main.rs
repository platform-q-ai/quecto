use cucumber::{World, given, then, when};
use quecto_mcp::{McpTool, build_registration, filter_tools, mcp_name_to_quecto_name};

#[derive(Debug, Default, World)]
struct McpWorld {
    tool: Option<McpTool>,
    tools: Vec<McpTool>,
    mapped_name: Option<String>,
    filtered: Vec<McpTool>,
    registration: Option<quecto_mcp::QuectoToolRegistration>,
    config: Option<quecto_mcp::Config>,
    error: Option<String>,
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

#[when(expr = "I filter tools with allowlist {string}")]
fn when_filter_allowlist(world: &mut McpWorld, allowlist: String) {
    world.filtered = filter_tools(&world.tools, &[], &split_csv(&allowlist), &[]).unwrap();
}

#[when(expr = "I filter tools with prefix {string} allowlist {string} and denylist {string}")]
fn when_filter_precedence(
    world: &mut McpWorld,
    prefix: String,
    allowlist: String,
    denylist: String,
) {
    world.filtered = filter_tools(
        &world.tools,
        &[prefix],
        &split_csv(&allowlist),
        &split_csv(&denylist),
    )
    .unwrap();
}

#[given("required quecto-mcp connection arguments")]
fn given_required_config_args(_world: &mut McpWorld) {}

#[when("I build a Quecto registration")]
fn when_build_registration(world: &mut McpWorld) {
    let tool = world.tool.as_ref().expect("tool");
    world.registration = Some(build_registration(tool).unwrap());
}

#[when(expr = "I build a Quecto registration with name prefix {string}")]
fn when_build_registration_with_prefix(world: &mut McpWorld, prefix: String) {
    let tool = world.tool.as_ref().expect("tool");
    world.registration =
        Some(quecto_mcp::build_registration_with_name_prefix(tool, &prefix).unwrap());
}

#[when(expr = "I try to build a Quecto registration with name prefix {string}")]
fn when_try_build_registration_with_prefix(world: &mut McpWorld, prefix: String) {
    let tool = world.tool.as_ref().expect("tool");
    world.error = quecto_mcp::build_registration_with_name_prefix(tool, &prefix)
        .unwrap_err()
        .to_string()
        .into();
}

#[when("I build Quecto tool registrations for the discovered tools")]
fn when_try_build_registrations(world: &mut McpWorld) {
    world.error = quecto_mcp::build_registrations(&world.tools)
        .unwrap_err()
        .to_string()
        .into();
}

#[when("I parse the quecto-mcp configuration")]
fn when_parse_config(world: &mut McpWorld) {
    world.config = Some(
        quecto_mcp::Config::from_env_and_args([
            "quecto-mcp".to_string(),
            "--socket".to_string(),
            "/tmp/quecto.sock".to_string(),
            "--mcp-url".to_string(),
            "https://perme8.example.test/mcp".to_string(),
            "--mcp-token".to_string(),
            "agent-token".to_string(),
        ])
        .unwrap(),
    );
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

#[then("the configured tool prefixes should be:")]
fn then_configured_prefixes(world: &mut McpWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("table");
    let expected: Vec<String> = table
        .rows
        .iter()
        .skip(1)
        .map(|row| row[0].clone())
        .collect();
    assert_eq!(
        world.config.as_ref().expect("config").tool_prefixes,
        expected
    );
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

#[then("quecto-mcp should reject the MCP tool configuration")]
fn then_rejected(world: &mut McpWorld) {
    assert!(world.error.is_some());
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

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[tokio::main]
async fn main() {
    McpWorld::cucumber()
        .max_concurrent_scenarios(1)
        .filter_run("tests/features", |_feat, _rule, _sc| true)
        .await;
}
