use super::*;

#[test]
fn test_definition() {
    let tool = WebSearchTool::new(None);
    let def = tool.definition();
    assert_eq!(def.name, "web_search");
    assert!(def.description.contains("Search"));
}

#[test]
fn test_encode_query_param() {
    assert_eq!(encode_query_param("hello world"), "hello+world");
    assert_eq!(encode_query_param("rust lang"), "rust+lang");
    assert_eq!(encode_query_param("a&b=c"), "a%26b%3Dc");
}

#[tokio::test]
async fn test_missing_query() {
    let tool = WebSearchTool::new(None);
    let result = tool.execute(r#"{"wrong":"field"}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_json() {
    let tool = WebSearchTool::new(None);
    let result = tool.execute("not json").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_brave_search_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "web": {
            "results": [
                {
                    "title": "Rust Programming",
                    "url": "https://rust-lang.org",
                    "description": "A systems language"
                },
                {
                    "title": "Rust Book",
                    "url": "https://doc.rust-lang.org/book/",
                    "description": "The Rust Programming Language"
                }
            ]
        }
    });

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool =
        WebSearchTool::with_base_urls(Some("test-key".to_string()), &server.uri(), "http://unused");
    let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Rust Programming"));
    assert!(result.content.contains("rust-lang.org"));
    assert!(result.content.contains("Rust Book"));
}

#[tokio::test]
async fn test_brave_search_oversized_output_is_bounded() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "web": {"results": [{
            "title": "Oversized",
            "url": "https://example.com/oversized",
            "description": "a".repeat(MAX_WEB_SEARCH_OUTPUT_BYTES * 2)
        }]}
    });

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool =
        WebSearchTool::with_base_urls(Some("test-key".to_string()), &server.uri(), "http://unused");
    let result = tool.execute(r#"{"query":"oversized"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.len() <= MAX_WEB_SEARCH_OUTPUT_BYTES);
    assert!(result.content.contains(&format!(
        "{}…",
        "a".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS - 1)
    )));
}

#[tokio::test]
async fn test_brave_search_exact_cap_is_not_marked_truncated() {
    let content = "a".repeat(MAX_WEB_SEARCH_OUTPUT_BYTES);
    let result = truncate_web_search_output(&content);

    assert_eq!(result.len(), MAX_WEB_SEARCH_OUTPUT_BYTES);
    assert_eq!(result, content);
    assert!(!result.ends_with(WEB_SEARCH_TRUNCATION_MARKER));
}

#[tokio::test]
async fn test_brave_search_honors_max_results_and_truncates_each_description() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "web": {"results": (0..4).map(|i| serde_json::json!({
            "title": format!("Result {i}"),
            "url": format!("https://example.com/{i}"),
            "description": "d".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS + 20)
        })).collect::<Vec<_>>()}
    });

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls_and_limits(
        Some("test-key".to_string()),
        &server.uri(),
        "http://unused",
        2,
        5,
    );
    let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("1. Result 0"));
    assert!(result.content.contains("2. Result 1"));
    assert!(!result.content.contains("3. Result 2"));
    assert!(result.content.contains(&format!(
        "{}…",
        "d".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS - 1)
    )));
}

#[tokio::test]
async fn test_ddg_search_honors_max_results_and_truncates_each_topic() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "",
        "AbstractURL": "",
        "RelatedTopics": (0..4).map(|i| serde_json::json!({
            "Text": format!("{} topic {i}", "t".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS + 20)),
            "FirstURL": format!("https://example.com/{i}")
        })).collect::<Vec<_>>()
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls_and_limits(None, "http://unused", &server.uri(), 5, 2);
    let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("1. "));
    assert!(result.content.contains("2. "));
    assert!(!result.content.contains("3. "));
    assert!(result.content.contains(&format!(
        "{}…",
        "t".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS - 1)
    )));
}

#[tokio::test]
async fn test_search_error_output_is_bounded() {
    let content = format!(
        "Search failed: {}",
        "e".repeat(MAX_WEB_SEARCH_OUTPUT_BYTES * 2)
    );
    let result = truncate_web_search_output(&content);
    assert!(result.len() <= MAX_WEB_SEARCH_OUTPUT_BYTES);
    assert!(result.ends_with(WEB_SEARCH_TRUNCATION_MARKER));
}

#[tokio::test]
async fn test_brave_search_no_results() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({"web": {"results": []}});

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool =
        WebSearchTool::with_base_urls(Some("test-key".to_string()), &server.uri(), "http://unused");
    // Empty results array should produce empty string (no items to iterate)
    let result = tool.execute(r#"{"query":"nothing"}"#).await.unwrap();
    assert!(!result.is_error);
}

#[tokio::test]
async fn test_brave_search_server_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let tool =
        WebSearchTool::with_base_urls(Some("test-key".to_string()), &server.uri(), "http://unused");
    let result = tool.execute(r#"{"query":"fail"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Search failed"));
}

#[tokio::test]
async fn test_ddg_search_abstract() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "Rust is a systems programming language.",
        "AbstractURL": "https://en.wikipedia.org/wiki/Rust",
        "RelatedTopics": []
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(
        result
            .content
            .contains("Rust is a systems programming language")
    );
    assert!(result.content.contains("wikipedia.org"));
}

#[tokio::test]
async fn test_ddg_search_oversized_abstract_is_bounded() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "b".repeat(MAX_WEB_SEARCH_OUTPUT_BYTES * 2),
        "AbstractURL": "https://example.com/source",
        "RelatedTopics": []
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.execute(r#"{"query":"oversized"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.len() <= MAX_WEB_SEARCH_OUTPUT_BYTES);
    assert!(result.content.ends_with(WEB_SEARCH_TRUNCATION_MARKER));
}

#[tokio::test]
async fn test_ddg_search_related_topics() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "",
        "AbstractURL": "",
        "RelatedTopics": [
            {"Text": "Rust (programming language)", "FirstURL": "https://example.com/rust"},
            {"Text": "Iron oxide", "FirstURL": "https://example.com/iron"}
        ]
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Rust (programming language)"));
    assert!(result.content.contains("Iron oxide"));
}

#[tokio::test]
async fn test_ddg_search_oversized_related_topics_are_bounded_at_utf8_boundary() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "",
        "AbstractURL": "",
        "RelatedTopics": [{
            "Text": "🦀".repeat(MAX_WEB_SEARCH_OUTPUT_BYTES),
            "FirstURL": "https://example.com/crab"
        }]
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.execute(r#"{"query":"crab"}"#).await.unwrap();

    assert!(!result.is_error);
    assert!(result.content.len() <= MAX_WEB_SEARCH_OUTPUT_BYTES);
    assert!(result.content.contains(&format!(
        "{}…",
        "🦀".repeat(MAX_WEB_SEARCH_RESULT_TEXT_CHARS - 1)
    )));
    assert!(!result.content.contains('\u{FFFD}'));
}

#[tokio::test]
async fn test_ddg_search_no_results() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "AbstractText": "",
        "RelatedTopics": []
    });

    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.execute(r#"{"query":"xyzzy"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("No results found"));
}

// --- #209: Shared reqwest::Client ---

#[test]
fn test_web_search_accepts_shared_client() {
    let client = reqwest::Client::new();
    let tool = WebSearchTool::with_client(None, client);
    assert_eq!(tool.definition().name, "web_search");
}
