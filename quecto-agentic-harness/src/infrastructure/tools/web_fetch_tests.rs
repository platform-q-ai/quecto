use super::*;

#[test]
fn test_definition() {
    let tool = WebFetchTool::new(32);
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
fn test_strip_html_removes_script_case_insensitive() {
    let html = "<SCRIPT>bad</SCRIPT>good";
    let result = strip_html(html);
    assert!(!result.contains("bad"));
    assert!(result.contains("good"));
}

#[test]
fn test_strip_html_removes_script_with_attributes() {
    let html = r#"<script type="text/javascript">bad</script>good"#;
    let result = strip_html(html);
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

    let tool = WebFetchTool::new(32);
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
async fn test_fetch_accepts_shared_client() {
    let tool = WebFetchTool::with_client(reqwest::Client::new(), 32);
    assert_eq!(tool.definition().name.as_ref(), "web_fetch");
}

#[derive(Debug)]
struct FixedResolver {
    candidates: Vec<SocketAddr>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl DestinationResolver for FixedResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolutionFuture<'a> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let candidates = self.candidates.clone();
        Box::pin(async move { Ok(candidates) })
    }
}

fn fixed_tool(
    host: &str,
    port: u16,
    candidates: Vec<SocketAddr>,
) -> (WebFetchTool, Arc<std::sync::atomic::AtomicUsize>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let resolver = Arc::new(FixedResolver {
        candidates,
        calls: Arc::clone(&calls),
    });
    let policy = WebDestinationPolicy::with_test_destination(host, port);
    (
        WebFetchTool::with_test_dependencies(32, policy, resolver),
        calls,
    )
}

#[tokio::test]
async fn controlled_resolver_pins_the_authorized_candidate_without_re_resolving() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pinned"))
        .mount(&server)
        .await;
    let port = server.address().port();
    let (tool, calls) = fixed_tool("public.test", port, vec![*server.address()]);

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert_eq!(result.content, "pinned");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "resolution must happen exactly once"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn resolver_with_only_denied_candidates_sends_zero_requests() {
    use wiremock::MockServer;
    let server = MockServer::start().await;
    let port = server.address().port();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let resolver = Arc::new(FixedResolver {
        candidates: vec![*server.address()],
        calls,
    });
    let tool = WebFetchTool::with_test_dependencies(32, WebDestinationPolicy::new(), resolver);

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        0,
        "a denied candidate must receive no request"
    );
}

#[test]
fn mixed_dns_answers_expose_only_the_affirmatively_authorized_subset() {
    let policy = WebDestinationPolicy::new();
    let url = reqwest::Url::parse("https://example.com/").unwrap();
    let allowed: SocketAddr = "8.8.8.8:443".parse().unwrap();
    let denied: SocketAddr = "127.0.0.1:443".parse().unwrap();

    let candidates = authorized_candidates(&policy, &url, vec![denied, allowed]);

    assert_eq!(candidates, vec![allowed]);
}

#[tokio::test]
async fn redirect_target_is_authorized_before_the_denied_server_receives_a_request() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let allowed = MockServer::start().await;
    let denied = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", denied.uri()))
        .mount(&allowed)
        .await;
    let port = allowed.address().port();
    let (tool, _) = fixed_tool("public.test", port, vec![*allowed.address()]);

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(allowed.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        denied.received_requests().await.unwrap().len(),
        0,
        "redirect denial must happen before send"
    );
}
