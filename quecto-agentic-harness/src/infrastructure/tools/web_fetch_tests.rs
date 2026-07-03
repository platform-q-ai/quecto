use super::*;

#[test]
fn test_definition() {
    let tool = WebFetchTool::new();
    let def = tool.definition();
    assert_eq!(def.name.as_ref(), "web_fetch");
    assert!(def.description.contains("Fetch"));
}

// ─── HTML stripping ──────────────────────────────────────────────────────

#[test]
fn test_strip_html_basic() {
    let html = "<p>Hello <b>world</b></p>";
    let text = strip_html(html);
    assert!(text.contains("Hello world"), "got: {text}");
}

#[test]
fn test_strip_html_removes_script() {
    let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
    let text = strip_html(html);
    assert!(text.contains("Before"));
    assert!(text.contains("After"));
    assert!(!text.contains("alert"));
}

#[test]
fn test_strip_html_removes_style() {
    let html = "<style>.foo { color: red; }</style><p>Content</p>";
    let text = strip_html(html);
    assert!(!text.contains("color"));
    assert!(text.contains("Content"));
}

#[test]
fn test_strip_html_removes_noscript() {
    let html = "<p>Before</p><noscript>Hidden</noscript><p>After</p>";
    let text = strip_html(html);
    assert!(text.contains("Before"));
    assert!(text.contains("After"));
    assert!(!text.contains("Hidden"));
}

#[test]
fn test_strip_html_removes_nav_footer_header() {
    let html = "<nav>Menu</nav><main>Content</main><footer>Copyright</footer>";
    let text = strip_html(html);
    assert!(!text.contains("Menu"));
    assert!(text.contains("Content"));
    assert!(!text.contains("Copyright"));
}

#[test]
fn test_strip_html_block_newlines() {
    let html = "<p>First</p><p>Second</p>";
    let text = strip_html(html);
    assert!(text.contains("First\n"), "got: {text:?}");
    assert!(text.contains("Second"));
}

#[test]
fn test_strip_html_br() {
    let html = "Line 1<br>Line 2<br/>Line 3";
    let text = strip_html(html);
    assert!(text.contains("Line 1\n"), "got: {text:?}");
    assert!(text.contains("Line 2\n"), "got: {text:?}");
}

#[test]
fn test_strip_html_collapses_whitespace() {
    let html = "<p>  lots   of    spaces  </p>";
    let text = strip_html(html);
    assert_eq!(text, "lots of spaces");
}

#[test]
fn test_strip_html_multiline_collapse() {
    let html = "<p>A</p>\n\n\n\n\n<p>B</p>";
    let text = strip_html(html);
    assert!(!text.contains("\n\n\n"), "got: {text:?}");
}

#[test]
fn test_strip_html_list_items() {
    let html = "<ul><li>One</li><li>Two</li><li>Three</li></ul>";
    let text = strip_html(html);
    assert!(text.contains("One"));
    assert!(text.contains("Two"));
    assert!(text.contains("Three"));
}

#[test]
fn test_strip_html_headings() {
    let html = "<h1>Title</h1><p>Paragraph</p>";
    let text = strip_html(html);
    assert!(text.contains("Title"));
    assert!(text.contains("Paragraph"));
}

#[test]
fn test_strip_html_plain_text_passthrough() {
    let text = strip_html("Just plain text, no HTML.");
    assert_eq!(text, "Just plain text, no HTML.");
}

#[test]
fn test_strip_html_removes_configured_tags_case_insensitively() {
    let html = "<HEADER>Top</HEADER><p>Keep</p><NoScript>Hidden</NoScript><NAV>Menu</NAV>";
    let text = strip_html(html);
    assert_eq!(text, "Keep");
}

#[test]
fn test_remove_tag_blocks_case_insensitive() {
    let html = "<SCRIPT>bad</SCRIPT>good";
    let result = remove_tag_blocks(html, "script");
    assert!(!result.contains("bad"));
    assert!(result.contains("good"));
}

#[test]
fn test_remove_tag_blocks_with_attributes() {
    let html = r#"<script type="text/javascript">bad</script>good"#;
    let result = remove_tag_blocks(html, "script");
    assert!(!result.contains("bad"));
    assert!(result.contains("good"));
}

// ─── Entity decoding ─────────────────────────────────────────────────────

#[test]
fn test_decode_entity_named() {
    assert_eq!(decode_entity("amp"), Some('&'));
    assert_eq!(decode_entity("lt"), Some('<'));
    assert_eq!(decode_entity("gt"), Some('>'));
    assert_eq!(decode_entity("quot"), Some('"'));
    assert_eq!(decode_entity("apos"), Some('\''));
    assert_eq!(decode_entity("nbsp"), Some(' '));
}

#[test]
fn test_decode_entity_numeric() {
    assert_eq!(decode_entity("#65"), Some('A'));
    assert_eq!(decode_entity("#x41"), Some('A'));
    assert_eq!(decode_entity("#x2603"), Some('☃'));
}

#[test]
fn test_decode_entities_in_text() {
    assert_eq!(decode_entities("&amp; &lt; &gt;"), "& < >");
    assert_eq!(decode_entities("hello&nbsp;world"), "hello world");
    assert_eq!(decode_entities("&#65;"), "A");
}

#[test]
fn test_tags_to_text_preserves_non_ascii() {
    let html = "<p>café résumé naïve</p>";
    assert_eq!(tags_to_text(html), "\ncafé résumé naïve\n");
}

#[test]
fn test_decode_entities_preserves_non_ascii() {
    assert_eq!(decode_entities("&#233;"), "é");
    assert_eq!(decode_entities("café"), "café");
}

#[test]
fn test_truncate_utf8_ascii() {
    assert_eq!(truncate_utf8("hello world", 5), "hello");
}

#[test]
fn test_truncate_utf8_boundary() {
    let s = "café";
    assert_eq!(truncate_utf8(s, 4), "caf");
    assert_eq!(truncate_utf8(s, 5), "café");
}

#[test]
fn test_truncate_utf8_no_truncation() {
    assert_eq!(truncate_utf8("short", 100), "short");
}

// ─── SSRF protection ─────────────────────────────────────────────────────

#[test]
fn test_is_restricted_host_or_ip() {
    assert!(is_restricted_host_or_ip("localhost"));
    assert!(is_restricted_host_or_ip("metadata.google.internal"));
    assert!(!is_restricted_host_or_ip("example.com"));
    assert!(is_restricted_host_or_ip("127.0.0.1"));
    assert!(is_restricted_host_or_ip("[::1]"));
    assert!(is_restricted_host_or_ip("::1"));
    assert!(is_restricted_host_or_ip("10.0.0.1"));
    assert!(is_restricted_host_or_ip("172.16.0.1"));
    assert!(is_restricted_host_or_ip("192.168.1.1"));
    assert!(is_restricted_host_or_ip("169.254.169.254"));
    assert!(!is_restricted_host_or_ip("8.8.8.8"));
}

#[test]
fn test_is_restricted_ip() {
    use std::net::{Ipv4Addr, Ipv6Addr};
    assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
    assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
    assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
    assert!(is_restricted_ip(IpAddr::V4(Ipv4Addr::new(
        169, 254, 169, 254
    ))));
    assert!(is_restricted_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert!(!is_restricted_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
}

#[tokio::test]
async fn test_ssrf_restricted_urls() {
    let cases = [
        ("http://localhost/secret", "localhost"),
        ("http://127.0.0.1/secret", "loopback IP"),
        ("http://169.254.169.254/latest/meta-data/", "AWS metadata"),
        ("http://10.0.0.1/internal", "private RFC1918"),
        ("http://[::1]/secret", "IPv6 loopback"),
        (
            "http://metadata.google.internal/computeMetadata/v1/",
            "Google metadata",
        ),
    ];

    let tool = WebFetchTool::new();
    for (url, label) in cases {
        let result = tool
            .execute(&format!(r#"{{"url":"{url}"}}"#))
            .await
            .unwrap_or_else(|e| panic!("{label}: tool execution failed: {e}"));
        assert!(result.is_error, "{label}: request should be rejected");
        assert!(
            result.content.contains("restricted"),
            "{label}: expected 'restricted' in error, got: {}",
            result.content
        );
    }
}

// ─── Tool execution ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_missing_url() {
    let tool = WebFetchTool::new();
    let result = tool.execute(r#"{"wrong":"field"}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_json() {
    let tool = WebFetchTool::new();
    let result = tool.execute("not json").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_scheme() {
    let tool = WebFetchTool::new();
    let result = tool
        .execute(r#"{"url":"ftp://example.com"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Invalid URL scheme"));
}

#[tokio::test]
async fn test_invalid_scheme_file() {
    let tool = WebFetchTool::new();
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

    let tool = WebFetchTool::new_allow_localhost(32);
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

    let tool = WebFetchTool::new_allow_localhost(32);
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

    let tool = WebFetchTool::new_allow_localhost(32);
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

    let tool = WebFetchTool::new_allow_localhost(1); // 1KB cap
    let result = tool
        .execute(&format!(r#"{{"url":"{}","raw":true}}"#, server.uri()))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.len() < 2048);
    assert!(result.content.contains("[Truncated"));
}

#[tokio::test]
async fn test_fetch_accepts_shared_client() {
    let tool = WebFetchTool::new_allow_localhost(32);
    assert_eq!(tool.definition().name.as_ref(), "web_fetch");
}
