use super::*;

// Web Fetch Tool BDD Steps
// ===========================================================================
//
// Uses wiremock to mock HTTP responses. The web_fetch tool is registered
// directly into the tool registry, bypassing the extension system.

use quecto::infrastructure::tools::web_fetch::WebFetchTool;

/// Leaked wiremock server for web_fetch BDD (stored in world).
fn start_web_fetch_mock() -> (&'static wiremock::MockServer, String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    let leaked = Box::leak(Box::new(server));
    (leaked, uri)
}

fn mount_web_fetch_mock(server: &'static wiremock::MockServer, mock: wiremock::Mock) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(mock.mount(server));
}

// ─── Given steps ─────────────────────────────────────────────────────────────

#[given("a tool workspace with a web_fetch tool backed by a mock server")]
fn given_web_fetch_workspace(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let mut registry = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);

    let (server, uri) = start_web_fetch_mock();
    // Create tool with allow_restricted_hosts=false (SSRF tests use real URLs,
    // not the mock server, so SSRF blocking is exercised directly).
    let tool = WebFetchTool::with_client(reqwest::Client::new(), 32);
    registry.register(Arc::new(tool));

    world.tool_workspace = Some(ws);
    world.tool_registry = Some(registry);
    world._tool_workspace_tmp = Some(td);
    world._web_fetch_mock_server = Some(server);
    world._web_fetch_mock_uri = Some(uri);
}

#[given("a tool workspace with a web_fetch tool backed by a mock server with 1KB limit")]
fn given_web_fetch_workspace_1kb(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let mut registry = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);

    let (server, uri) = start_web_fetch_mock();
    let tool = WebFetchTool::with_client(reqwest::Client::new(), 1);
    registry.register(Arc::new(tool));

    world.tool_workspace = Some(ws);
    world.tool_registry = Some(registry);
    world._tool_workspace_tmp = Some(td);
    world._web_fetch_mock_server = Some(server);
    world._web_fetch_mock_uri = Some(uri);
}

#[given("the mock web server returns HTML:")]
fn given_mock_returns_html(world: &mut QuectoWorld, step: &gherkin::Step) {
    let body = step
        .docstring
        .as_ref()
        .expect("step should have a docstring")
        .clone();
    let server = world._web_fetch_mock_server.expect("mock server not set");
    mount_web_fetch_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/html"),
        ),
    );
}

#[given("the mock web server returns body:")]
fn given_mock_returns_body(world: &mut QuectoWorld, step: &gherkin::Step) {
    let body = step
        .docstring
        .as_ref()
        .expect("step should have a docstring")
        .clone();
    let server = world._web_fetch_mock_server.expect("mock server not set");
    mount_web_fetch_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/plain"),
        ),
    );
}

#[given("the mock web server returns a 4KB plain text body")]
fn given_mock_returns_4kb(world: &mut QuectoWorld) {
    let body = "A".repeat(4096);
    let server = world._web_fetch_mock_server.expect("mock server not set");
    mount_web_fetch_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET")).respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_string(body)
                .insert_header("content-type", "text/plain"),
        ),
    );
}

#[given("the mock web server returns HTTP 404")]
fn given_mock_returns_404(world: &mut QuectoWorld) {
    let server = world._web_fetch_mock_server.expect("mock server not set");
    mount_web_fetch_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(404)),
    );
}

#[given("the mock web server returns HTTP 500")]
fn given_mock_returns_500(world: &mut QuectoWorld) {
    let server = world._web_fetch_mock_server.expect("mock server not set");
    mount_web_fetch_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(500)),
    );
}

// ─── When steps ──────────────────────────────────────────────────────────────

#[when("the agent executes tool \"web_fetch\" with mock URL")]
fn when_execute_web_fetch_mock_url(world: &mut QuectoWorld) {
    let uri = world
        ._web_fetch_mock_uri
        .as_ref()
        .expect("mock URI not set")
        .clone();
    let args = serde_json::json!({"url": uri}).to_string();
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("web_fetch", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when("the agent executes tool \"web_fetch\" with mock URL and raw mode")]
fn when_execute_web_fetch_mock_raw(world: &mut QuectoWorld) {
    let uri = world
        ._web_fetch_mock_uri
        .as_ref()
        .expect("mock URI not set")
        .clone();
    let args = serde_json::json!({"url": uri, "raw": true}).to_string();
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("web_fetch", &args));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when(expr = "the agent executes tool \"web_fetch\" with raw args {string}")]
fn when_execute_web_fetch_raw_args(world: &mut QuectoWorld, raw: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute("web_fetch", &raw));
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

// ─── Then steps ──────────────────────────────────────────────────────────────

// NOTE: "the tool result should not contain {string}" step lives in
// sandbox_steps.rs — do NOT duplicate it here (causes Cucumber ambiguity, #428).

#[then("the tool result should be a domain error")]
fn then_tool_result_is_domain_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    assert!(
        result.is_err(),
        "expected DomainError, got Ok: {:?}",
        result.as_ref().unwrap().content
    );
}
