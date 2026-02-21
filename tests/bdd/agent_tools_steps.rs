use super::*;

// Agent Tools Steps
// ===========================================================================

#[given("a tool workspace")]
fn given_tool_workspace(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);
    world.tool_workspace = Some(ws);
    world.tool_registry = Some(registry);
    world._temp_dir = Some(td);
}

#[given(expr = "a file {string} exists with content {string}")]
fn given_file_exists(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, &content).expect("write file");
}

#[when(expr = "the agent executes tool {string} with args:")]
fn when_agent_executes_tool(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    let args_json = table_to_json(table);

    let registry = world.tool_registry.as_ref().expect("tool registry not set");

    // Run the tool using a tokio runtime
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, &args_json));

    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the tool result should contain {string}")]
fn then_tool_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            tr.content.contains(&expected),
            "expected tool result to contain '{}', got: {}",
            expected,
            tr.content
        ),
        Err(e) => panic!("tool returned error: {}", e),
    }
}

#[then("the tool result should not be an error")]
fn then_tool_result_not_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.is_error,
            "expected tool result to not be an error, content: {}",
            tr.content
        ),
        Err(e) => panic!("tool returned DomainError: {}", e),
    }
}

#[then(expr = "the file {string} should exist in the workspace")]
fn then_file_exists_in_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    assert!(
        path.exists(),
        "file '{}' should exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the file {string} should contain {string}")]
fn then_file_contains(world: &mut QuectoWorld, filename: String, expected: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    assert!(
        content.contains(&expected),
        "expected '{}' to contain '{}', got: {}",
        filename,
        expected,
        content
    );
}

#[then(expr = "the tool registry should contain {string}")]
fn then_registry_contains(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let names = registry.names();
    assert!(
        names.contains(&tool_name),
        "registry should contain '{}', has: {:?}",
        tool_name,
        names
    );
}

// ===========================================================================
// Security (Subagent/Heartbeat Inheritance) Steps
// ===========================================================================

#[given("a subagent context inheriting restrict_to_workspace")]
fn given_subagent_inheriting_sandbox(world: &mut QuectoWorld) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Create a subagent config that inherits the sandbox's restrict_to_workspace
    world.subagent_config = Some(SubagentConfig {
        task: "test task".to_string(),
        agent_id: None,
        restrict_to_workspace: sb.restrict_to_workspace,
        deliver_to: None,
        system: None,
    });
    let ctx = SubagentContext::from_config(world.subagent_config.as_ref().unwrap());
    world.subagent_context = Some(ctx);
}

#[when(expr = "the subagent sandbox validates path {string}")]
fn when_subagent_validates_path(world: &mut QuectoWorld, path: String) {
    // The subagent inherits the same sandbox config; validate using it
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Verify the subagent context also has restrict_to_workspace set
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not set");
    assert_eq!(ctx.restrict_to_workspace, sb.restrict_to_workspace);
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "a heartbeat sandbox validates path {string}")]
fn when_heartbeat_validates_path(world: &mut QuectoWorld, path: String) {
    // Heartbeat tasks run within the same sandbox restrictions
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

// ===========================================================================
// Web Search Steps
// ===========================================================================

/// Helper: start a wiremock server, leak it, return static ref and URI.
fn start_leaked_mock_server() -> (&'static wiremock::MockServer, String) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (server_ref, uri) = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let uri = server.uri();
        let leaked: &'static wiremock::MockServer = Box::leak(Box::new(server));
        (leaked, uri)
    });
    std::mem::forget(rt);
    (server_ref, uri)
}

/// Helper: mount a mock on a leaked wiremock server.
fn mount_mock(server: &'static wiremock::MockServer, mock: wiremock::Mock) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(mock.mount(server));
    // rt can be safely dropped — server is already leaked via Box::leak
}

#[given("a web search tool configured with a mock DuckDuckGo API")]
fn given_web_search_ddg_mock(world: &mut QuectoWorld) {
    let (server_ref, uri) = start_leaked_mock_server();

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &uri);
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    registry.register(Arc::new(tool));

    world.web_search_mock_server = Some(server_ref);
    world.web_search_used_ddg = true;
}

#[given(expr = "a web search tool configured with a mock Brave Search API and api_key {string}")]
fn given_web_search_brave_mock(world: &mut QuectoWorld, api_key: String) {
    let (server_ref, uri) = start_leaked_mock_server();

    let tool = WebSearchTool::with_base_urls(Some(api_key), &uri, "http://unused");
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    registry.register(Arc::new(tool));

    world.web_search_mock_server = Some(server_ref);
    world.web_search_used_ddg = false;
}

#[given("a web search tool configured with no Brave API key")]
fn given_web_search_no_brave_key(world: &mut QuectoWorld) {
    // No Brave key → DDG fallback. Mock will be set up in the next step.
    world.web_search_used_ddg = true;
}

#[given("a mock DuckDuckGo API that returns results")]
fn given_mock_ddg_returns_results(world: &mut QuectoWorld) {
    let (server_ref, uri) = start_leaked_mock_server();

    let response = serde_json::json!({
        "AbstractText": "",
        "AbstractURL": "",
        "RelatedTopics": [
            {
                "Text": "DuckDuckGo result",
                "FirstURL": "https://ddg.example.com/result"
            }
        ]
    });
    mount_mock(
        server_ref,
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response)),
    );

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &uri);
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    registry.register(Arc::new(tool));

    world.web_search_mock_server = Some(server_ref);
}

#[given(expr = "the mock search API returns results for {string}:")]
fn given_mock_search_returns_results(
    world: &mut QuectoWorld,
    _query: String,
    step: &gherkin::Step,
) {
    let table = step.table.as_ref().expect("step should have a table");
    let server = world
        .web_search_mock_server
        .expect("web search mock server not set");

    // Parse table rows (skip header row)
    let mut items = Vec::new();
    for row in table.rows.iter().skip(1) {
        if row.len() >= 2 {
            items.push((row[0].trim().to_string(), row[1].trim().to_string()));
        }
    }

    if world.web_search_used_ddg {
        let topics: Vec<serde_json::Value> = items
            .iter()
            .map(|(title, url)| serde_json::json!({"Text": title, "FirstURL": url}))
            .collect();
        let response = serde_json::json!({
            "AbstractText": "",
            "AbstractURL": "",
            "RelatedTopics": topics
        });
        mount_mock(
            server,
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response)),
        );
    } else {
        let results: Vec<serde_json::Value> = items
            .iter()
            .map(|(title, url)| serde_json::json!({"title": title, "url": url, "description": ""}))
            .collect();
        let response = serde_json::json!({"web": {"results": results}});
        mount_mock(
            server,
            wiremock::Mock::given(wiremock::matchers::method("GET"))
                .and(wiremock::matchers::path("/res/v1/web/search"))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response)),
        );
    }
}

#[given(expr = "the mock Brave API returns results for {string}:")]
fn given_mock_brave_returns_results(world: &mut QuectoWorld, _query: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    let server = world
        .web_search_mock_server
        .expect("web search mock server not set");

    let mut items = Vec::new();
    for row in table.rows.iter().skip(1) {
        if row.len() >= 2 {
            items.push((row[0].trim().to_string(), row[1].trim().to_string()));
        }
    }

    let results: Vec<serde_json::Value> = items
        .iter()
        .map(|(title, url)| serde_json::json!({"title": title, "url": url, "description": ""}))
        .collect();
    let response = serde_json::json!({"web": {"results": results}});
    mount_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/res/v1/web/search"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(&response)),
    );
}

#[given("the mock search API returns an HTTP 503 error")]
fn given_mock_search_returns_503(world: &mut QuectoWorld) {
    let server = world
        .web_search_mock_server
        .expect("web search mock server not set");
    mount_mock(
        server,
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(503)),
    );
}

#[then("the tool result should contain search results")]
fn then_tool_result_has_search_results(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => {
            assert!(!tr.is_error, "tool returned an error: {}", tr.content);
            assert!(
                !tr.content.is_empty() && tr.content != "No results found.",
                "expected search results, got: {}",
                tr.content
            );
        }
        Err(e) => panic!("tool returned DomainError: {}", e),
    }
}

#[then("the search should have used DuckDuckGo")]
fn then_search_used_ddg(world: &mut QuectoWorld) {
    // The DDG fallback is verified by the tool having been configured with
    // no Brave key (web_search_used_ddg flag) and producing results from DDG mock.
    assert!(
        world.web_search_used_ddg,
        "expected DDG search, but Brave was configured"
    );
    // Also verify that a result was returned (proving DDG mock was hit)
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.is_error,
            "search should have succeeded via DDG, got error: {}",
            tr.content
        ),
        Err(e) => panic!("search failed: {}", e),
    }
}
