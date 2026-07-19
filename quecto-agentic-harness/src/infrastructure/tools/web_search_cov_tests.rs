use super::*;

#[tokio::test]
async fn brave_parse_errors_and_missing_web_results_are_reported_without_network_leak() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let tool = WebSearchTool::with_base_urls(Some("k".into()), &server.uri(), "http://unused");
    let err = tool.search_brave("a b", "k").await.unwrap_err().to_string();
    assert!(err.contains("Failed to parse Brave"), "{err}");

    let server2 = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"other":[]})))
        .mount(&server2)
        .await;
    let tool = WebSearchTool::with_base_urls(Some("k".into()), &server2.uri(), "http://unused");
    assert_eq!(
        tool.search_brave("rust", "k").await.unwrap(),
        "No results found."
    );
}

#[tokio::test]
async fn ddg_related_topics_parse_error_and_no_results_paths() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"RelatedTopics":[{"Text":"One","FirstURL":"https://one"},{"NoText":true}]})))
        .mount(&server).await;
    let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
    let result = tool.search_ddg("rust & ferris").await.unwrap();
    assert!(result.contains("1. One (https://one)"));

    let bad = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("oops"))
        .mount(&bad)
        .await;
    let tool = WebSearchTool::with_base_urls(None, "http://unused", &bad.uri());
    assert!(
        tool.search_ddg("x")
            .await
            .unwrap_err()
            .to_string()
            .contains("Failed to parse DDG")
    );
}
