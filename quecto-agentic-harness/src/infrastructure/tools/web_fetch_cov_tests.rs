use super::*;
use crate::domain::tool::Tool;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn with_allowed_host_fetches_local_html_and_truncates_utf8() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<html><body><h1>Héllo</h1><script>bad()</script><p>world</p></body></html>",
        ))
        .mount(&server)
        .await;
    let uri = server.uri();
    let host = uri.trim_start_matches("http://");
    let tool = WebFetchTool::with_allowed_host(1, host);
    let result = tool
        .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("Héllo"));
    assert!(result.content.contains("world"));
    assert!(!result.content.contains("bad()"));
}

#[tokio::test]
async fn execute_rejects_restricted_host_and_oversized_body() {
    let tool = WebFetchTool::new();
    let blocked = tool
        .execute(r#"{"url":"http://127.0.0.1:9/"}"#)
        .await
        .unwrap();
    assert!(blocked.is_error);
    assert!(blocked.content.contains("Blocked"));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'a'; MAX_RAW_BYTES + 1]))
        .mount(&server)
        .await;
    let tool = WebFetchTool::new_allow_localhost(32);
    let err = tool
        .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Response too large"));
}

#[test]
fn html_helpers_cover_entities_and_restricted_addresses() {
    assert!(is_restricted_host_or_ip("[::1]"));
    assert!(is_restricted_host_or_ip("metadata.google.internal."));
    assert!(!is_restricted_host_or_ip("example.com"));
    assert_eq!(truncate_utf8("éclair", 1), "");
    assert_eq!(
        strip_html("<DIV>A&nbsp;&amp;&#x42;<br> C <broken"),
        "A &B\nC <broken"
    );
}

#[tokio::test]
async fn malformed_url_is_rejected_before_any_request() {
    let tool = WebFetchTool::new();
    // Not a parse-able URL at all: must fail at validation, not surface as a
    // network error (which would imply a request was attempted).
    let err = tool
        .execute(r#"{"url":"http://[not a url"}"#)
        .await
        .expect_err("a malformed URL must fail before any request is attempted");

    let msg = err.to_string();
    assert!(
        msg.contains("Invalid URL"),
        "expected the URL-parse message, got: {msg}"
    );
}
