use super::super::*;

// ─── Tool execution ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_missing_url() {
    let tool = WebFetchTool::new(32);
    let result = tool.execute(r#"{"wrong":"field"}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_json() {
    let tool = WebFetchTool::new(32);
    let result = tool.execute("not json").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_scheme() {
    let tool = WebFetchTool::new(32);
    let result = tool
        .execute(r#"{"url":"ftp://example.com"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Invalid URL scheme"));
}

#[tokio::test]
async fn test_invalid_scheme_file() {
    let tool = WebFetchTool::new(32);
    let result = tool
        .execute(r#"{"url":"file:///etc/passwd"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Invalid URL scheme"));
}

// ─── Wiremock integration ────────────────────────────────────────────────

#[tokio::test]
async fn test_fetch_html_strips_tags() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let html = r#"<html><head><title>Test</title><style>body{}</style></head>
            <body><nav>Menu</nav><h1>Hello</h1><p>World</p><footer>Foot</footer></body></html>"#;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(html))
        .mount(&server)
        .await;

    let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));
    let result = tool
        .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
        .await
        .unwrap();
    assert!(!result.is_error, "error: {}", result.content);
    assert!(result.content.contains("Hello"), "got: {}", result.content);
    assert!(result.content.contains("World"), "got: {}", result.content);
    assert!(!result.content.contains("Menu"), "nav not stripped");
    assert!(!result.content.contains("Foot"), "footer not stripped");
    assert!(!result.content.contains("body{}"), "style not stripped");
}

#[tokio::test]
async fn test_fetch_raw_mode() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let json_body = r#"{"key":"value","items":[1,2,3]}"#;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(json_body))
        .mount(&server)
        .await;

    let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));
    let result = tool
        .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, json_body);
}

#[tokio::test]
async fn test_fetch_http_error() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));
    let result = tool
        .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("404"));
}

#[tokio::test]
async fn test_fetch_truncates_large_response() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let big_body = "A".repeat(2048);

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(&big_body))
        .mount(&server)
        .await;

    let tool = WebFetchTool::with_allowed_host(1, server.uri().trim_start_matches("http://")); // 1KB cap
    let result = tool
        .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.len() < 2048);
    assert!(result.content.contains("[Truncated"));
}

#[tokio::test]
async fn test_fetch_accepts_replayable_client_recipe() {
    let tool = WebFetchTool::with_client_builder_factory(Arc::new(reqwest::Client::builder), 32);
    assert_eq!(tool.definition().name.as_ref(), "web_fetch");
}
