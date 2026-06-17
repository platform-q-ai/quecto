use super::*;
use tokio::net::UnixListener;
use wiremock::matchers::{body_partial_json, header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn maps_dotted_names_to_safe_names() {
    assert_eq!(
        mcp_name_to_quecto_name("community.channels.send_message").unwrap(),
        "community_channels_send_message"
    );
}

#[test]
fn detects_name_collisions() {
    let tools = vec![
        McpTool {
            name: "a.b".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        },
        McpTool {
            name: "a_b".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    assert!(matches!(
        build_mapping(&tools),
        Err(QuectoMcpError::ToolNameCollision { .. })
    ));
}

#[test]
fn config_defaults_to_community_prefix_and_supports_options() {
    let config = Config::from_env_and_args([
        "quecto-mcp".to_string(),
        "--socket".to_string(),
        "/tmp/q.sock".to_string(),
        "--mcp-url".to_string(),
        "https://example.test/mcp".to_string(),
        "--mcp-token".to_string(),
        "agent-token".to_string(),
        "--name-prefix".to_string(),
        "mcp_".to_string(),
        "--timeout".to_string(),
        "12".to_string(),
        "--register-timeout".to_string(),
        "3".to_string(),
    ])
    .unwrap();

    assert_eq!(config.tool_prefixes, vec!["community."]);
    assert_eq!(config.name_prefix, "mcp_");
    assert_eq!(config.timeout, Duration::from_secs(12));
    assert_eq!(config.register_timeout, Duration::from_secs(3));
    assert_eq!(config.refresh_interval, None);
}

#[test]
fn refresh_interval_is_rejected_until_refresh_is_implemented() {
    let err = Config::from_env_and_args([
        "quecto-mcp".to_string(),
        "--socket".to_string(),
        "/tmp/q.sock".to_string(),
        "--mcp-url".to_string(),
        "https://example.test/mcp".to_string(),
        "--mcp-token".to_string(),
        "agent-token".to_string(),
        "--refresh-interval".to_string(),
        "60".to_string(),
    ])
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("--refresh-interval is not implemented")
    );
}

#[test]
fn name_prefix_applies_to_registration_and_mapping() {
    let tools = vec![McpTool {
        name: "community.feed.list".into(),
        description: "".into(),
        input_schema: serde_json::json!({}),
    }];
    let registrations = build_registrations_with_name_prefix(&tools, "mcp_").unwrap();
    let mapping = build_mapping_with_name_prefix(&tools, "mcp_").unwrap();
    assert_eq!(registrations[0].name, "mcp_community_feed_list");
    assert_eq!(mapping["mcp_community_feed_list"], "community.feed.list");
}

#[test]
fn filters_tools() {
    let tools = vec![
        McpTool {
            name: "community.feed.list".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        },
        McpTool {
            name: "ticket.read".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
        },
    ];
    let filtered = filter_tools(&tools, &["community.".into()], &[], &[]).unwrap();
    assert_eq!(filtered.len(), 1);
}

#[tokio::test]
async fn mcp_client_lists_tools_and_calls_tool() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header_regex(
            "accept",
            "application/json.*text/event-stream",
        ))
        .and(body_partial_json(
            serde_json::json!({"method": "initialize"}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("mcp-session-id", "session-1")
                .set_body_json(serde_json::json!({"jsonrpc": "2.0", "id": "init", "result": {}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(
            serde_json::json!({"method": "notifications/initialized"}),
        ))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(
            serde_json::json!({"method": "tools/list"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": "list",
            "result": {"tools": [{
                "name": "community.feed.list",
                "description": "List feed posts",
                "inputSchema": {"type": "object"}
            }]}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(serde_json::json!({
            "method": "tools/call",
            "params": {"name": "community.feed.list", "arguments": {"limit": 1}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "result": {"content": [{"type": "text", "text": "feed item"}]}
        })))
        .mount(&server)
        .await;

    let client = McpClient::new(server.uri(), "secret-token".into());
    client.initialize("perme8-mcp").await.unwrap();
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools[0].name, "community.feed.list");
    assert_eq!(tools[0].description, "List feed posts");

    let result = client
        .call_tool("community.feed.list", serde_json::json!({"limit": 1}))
        .await
        .unwrap();
    assert_eq!(result, "feed item");
}

#[tokio::test]
async fn mcp_http_response_size_is_capped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(
            serde_json::json!({"method": "tools/call"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_MCP_RESPONSE_BYTES + 1)),
        )
        .mount(&server)
        .await;

    let client = McpClient::new(server.uri(), "secret-token".into());
    let err = client
        .call_tool("community.feed.list", serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, QuectoMcpError::ResponseTooLarge(_, _)));
}

#[tokio::test]
async fn mcp_json_rpc_errors_are_decoded_and_redacted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(
            serde_json::json!({"method": "tools/call"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "error": {"code": -32001, "message": "Authorization: Bearer secret denied"}
        })))
        .mount(&server)
        .await;

    let client = McpClient::new(server.uri(), "secret-token".into());
    let err = client
        .call_tool("community.feed.list", serde_json::json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("<redacted-header>"));
    assert!(err.contains("Bearer <redacted>"));
    assert!(!err.contains("secret"));
}

#[tokio::test]
async fn uds_extension_fails_when_register_tools_fails() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut register_line = String::new();
        reader.read_line(&mut register_line).await.unwrap();
        drop(reader);
        let response = serde_json::json!({
            "type": "response",
            "id": "quecto-mcp-register",
            "command": "register_tools",
            "success": false,
            "error": "tool 'bash' shadows a core tool"
        });
        let mut response_line = serde_json::to_string(&response).unwrap();
        response_line.push('\n');
        stream.write_all(response_line.as_bytes()).await.unwrap();
    });

    let tools = vec![McpTool {
        name: "community.chat.send_dm".into(),
        description: "Send a DM".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let err = serve_uds_extension(
        &socket,
        RegisteredMcpTools {
            registrations: build_registrations(&tools).unwrap(),
            mapping: build_mapping(&tools).unwrap(),
        },
        McpClient::new("http://127.0.0.1:9".into(), "token".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("shadows a core tool"));
    server_task.await.unwrap();
}

#[tokio::test]
async fn uds_extension_registers_tools_and_returns_tool_result() {
    let mcp_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_partial_json(serde_json::json!({
            "method": "tools/call",
            "params": {"name": "community.chat.send_dm", "arguments": {"message": "hi"}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call",
            "result": {"content": [{"type": "text", "text": "sent"}]}
        })))
        .mount(&mcp_server)
        .await;

    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut register_line = String::new();
        reader.read_line(&mut register_line).await.unwrap();
        let register_json: Value = serde_json::from_str(register_line.trim()).unwrap();
        assert_eq!(register_json["type"], "register_tools");
        assert_eq!(register_json["tools"][0]["name"], "community_chat_send_dm");
        drop(reader);
        let response = serde_json::json!({
            "type": "response",
            "id": "quecto-mcp-register",
            "command": "register_tools",
            "success": true
        });
        let mut response_line = serde_json::to_string(&response).unwrap();
        response_line.push('\n');
        stream.write_all(response_line.as_bytes()).await.unwrap();

        let execute = serde_json::json!({
            "type": "execute_tool",
            "toolCallId": "uds-1",
            "toolName": "community_chat_send_dm",
            "arguments": "{\"message\":\"hi\"}"
        });
        let mut line = serde_json::to_string(&execute).unwrap();
        line.push('\n');
        stream.write_all(line.as_bytes()).await.unwrap();

        let mut reader = BufReader::new(&mut stream);
        let mut result_line = String::new();
        reader.read_line(&mut result_line).await.unwrap();
        let result_json: Value = serde_json::from_str(result_line.trim()).unwrap();
        assert_eq!(result_json["type"], "tool_result");
        assert_eq!(result_json["toolCallId"], "uds-1");
        assert_eq!(result_json["content"], "sent");
        assert_eq!(result_json["isError"], false);
    });

    let tools = vec![McpTool {
        name: "community.chat.send_dm".into(),
        description: "Send a DM".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let registrations = build_registrations(&tools).unwrap();
    let mapping = build_mapping(&tools).unwrap();
    let mcp = McpClient::new(mcp_server.uri(), "token".into());
    serve_uds_extension(
        &socket,
        RegisteredMcpTools {
            registrations,
            mapping,
        },
        mcp,
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    server_task.await.unwrap();
}
