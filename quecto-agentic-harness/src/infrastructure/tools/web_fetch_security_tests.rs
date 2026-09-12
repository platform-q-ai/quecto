use super::*;

// ─── SSRF protection ─────────────────────────────────────────────────────

#[path = "web_fetch_behavior_tests.rs"]
mod behavior;

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
    let policy =
        WebDestinationPolicy::with_test_destination(host, port, "127.0.0.1".parse().unwrap());
    (
        WebFetchTool::with_test_dependencies(32, policy, resolver),
        calls,
    )
}

#[tokio::test]
async fn configured_client_recipe_preserves_custom_root_with_authorized_tls_pin() {
    use rustls::ServerConfig;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsAcceptor;

    const CA_CERTIFICATE: &[u8] = include_bytes!("testdata/public_test_ca.der");
    const SERVER_CERTIFICATE: &[u8] = include_bytes!("testdata/public_test_leaf.der");
    const PRIVATE_KEY: &[u8] = include_bytes!("testdata/public_test_key.der");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(SERVER_CERTIFICATE.to_vec())],
                PrivateKeyDer::try_from(PRIVATE_KEY.to_vec()).unwrap(),
            )
            .unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = TlsAcceptor::from(Arc::new(tls))
            .accept(stream)
            .await
            .unwrap();
        let mut request = [0_u8; 1024];
        let count = stream.read(&mut request).await.unwrap();
        assert!(
            std::str::from_utf8(&request[..count])
                .unwrap()
                .starts_with("GET / HTTP/1.1")
        );
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\ntrusted custom",
            )
            .await
            .unwrap();
    });

    let certificate = reqwest::Certificate::from_der(CA_CERTIFICATE).unwrap();
    let factory: ClientBuilderFactory = Arc::new(move || {
        reqwest::Client::builder()
            .use_rustls_tls()
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate.clone())
    });
    let resolver = Arc::new(FixedResolver {
        candidates: vec![address],
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    let policy =
        WebDestinationPolicy::with_test_destination("public.test", address.port(), address.ip());
    let tool = WebFetchTool::with_test_dependencies_and_client_factory(
        32,
        REQUEST_TIMEOUT,
        policy,
        resolver,
        factory,
    );

    let request = format!(
        r#"{{"url":"https://public.test:{}/","raw":true}}"#,
        address.port()
    );
    let result = tool
        .execute(&request)
        .await
        .unwrap_or_else(|error| panic!("configured TLS request failed: {error:?}"));

    assert_eq!(result.content, "trusted custom");
    assert!(!result.is_error);
    server.await.unwrap();
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
async fn mixed_dns_answers_execute_only_against_the_exact_allowed_candidate() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let allowed = MockServer::start().await;
    let port = allowed.address().port();
    let denied_listener = std::net::TcpListener::bind(("127.0.0.2", port)).unwrap();
    let denied = MockServer::builder()
        .listener(denied_listener)
        .start()
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("allowed candidate"))
        .mount(&allowed)
        .await;

    let denied_candidate = *denied.address();
    let resolver = Arc::new(FixedResolver {
        candidates: vec![denied_candidate, *allowed.address()],
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    let policy =
        WebDestinationPolicy::with_test_destination("public.test", port, allowed.address().ip());
    let tool = WebFetchTool::with_test_dependencies(32, policy, resolver);

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert_eq!(result.content, "allowed candidate");
    assert_eq!(allowed.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        denied.received_requests().await.unwrap().len(),
        0,
        "the denied answer must not be exposed to the connector"
    );
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

fn loopback_test_policy(host: &str, port: u16, candidate: IpAddr) -> WebDestinationPolicy {
    WebDestinationPolicy::with_test_destination(host, port, candidate)
}

fn fixed_tool_with_timeout(
    host: &str,
    port: u16,
    candidate: SocketAddr,
    request_timeout: Duration,
) -> WebFetchTool {
    let resolver = Arc::new(FixedResolver {
        candidates: vec![candidate],
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    WebFetchTool::with_test_dependencies_and_timeout(
        32,
        request_timeout,
        loopback_test_policy(host, port, candidate.ip()),
        resolver,
    )
}

#[tokio::test]
async fn only_the_redirect_status_allowlist_is_followed() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    for status in [300_u16, 304, 305, 306] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(ResponseTemplate::new(status).insert_header("Location", "/unexpected"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/unexpected"))
            .respond_with(ResponseTemplate::new(200).set_body_string("must not follow"))
            .mount(&server)
            .await;
        let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));

        let result = tool
            .execute(&format!(r#"{{"url":"{}/start"}}"#, server.uri()))
            .await
            .unwrap();

        assert!(
            result.is_error,
            "status {status} must remain a non-success response"
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "status {status} must not cause another request"
        );
        assert_eq!(requests[0].url.path(), "/start");
    }
}

#[tokio::test]
async fn multiple_location_headers_fail_closed_before_either_target() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .append_header("Location", "/first")
                .append_header("Location", "/second"),
        )
        .mount(&server)
        .await;
    for target in ["/first", "/second"] {
        Mock::given(method("GET"))
            .and(path(target))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));

    let result = tool
        .execute(&format!(r#"{{"url":"{}/start"}}"#, server.uri()))
        .await
        .unwrap();

    assert!(result.is_error);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "ambiguous redirect must not be followed");
    assert_eq!(requests[0].url.path(), "/start");
}

#[tokio::test]
async fn redirect_to_denied_scheme_stops_before_another_request() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "file:///etc/passwd"))
        .mount(&server)
        .await;
    let tool = WebFetchTool::with_allowed_host(32, server.uri().trim_start_matches("http://"));

    let result = tool
        .execute(&format!(r#"{{"url":"{}"}}"#, server.uri()))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.content.contains("Invalid URL scheme"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn multi_hop_redirect_stops_before_a_later_denied_destination() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let allowed = MockServer::start().await;
    let denied = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/first"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/second"))
        .mount(&allowed)
        .await;
    Mock::given(method("GET"))
        .and(path("/second"))
        .respond_with(ResponseTemplate::new(307).insert_header("Location", denied.uri()))
        .mount(&allowed)
        .await;
    let tool = WebFetchTool::with_allowed_host(32, allowed.uri().trim_start_matches("http://"));

    let result = tool
        .execute(&format!(r#"{{"url":"{}/first"}}"#, allowed.uri()))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(allowed.received_requests().await.unwrap().len(), 2);
    assert_eq!(denied.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn one_deadline_covers_all_redirect_hops_and_the_body() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/first"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "/second")
                .set_delay(Duration::from_millis(80)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/second"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("late body")
                .set_delay(Duration::from_millis(80)),
        )
        .mount(&server)
        .await;
    let port = server.address().port();
    let tool = fixed_tool_with_timeout(
        "public.test",
        port,
        *server.address(),
        Duration::from_millis(120),
    );
    let started = Instant::now();

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/first"}}"#))
        .await;

    let error = result.expect_err("combined hop delays must exceed the single deadline");
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < Duration::from_millis(220));
}

#[tokio::test]
async fn dropping_one_in_flight_fetch_does_not_cancel_a_concurrent_fetch() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("slow")
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/fast"))
        .respond_with(ResponseTemplate::new(200).set_body_string("independent"))
        .mount(&server)
        .await;
    let tool = Arc::new(WebFetchTool::with_allowed_host(
        32,
        server.uri().trim_start_matches("http://"),
    ));
    let slow_tool = Arc::clone(&tool);
    let slow_url = format!(r#"{{"url":"{}/slow"}}"#, server.uri());
    let slow = tokio::spawn(async move { slow_tool.execute(&slow_url).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    slow.abort();

    let fast = tool
        .execute(&format!(r#"{{"url":"{}/fast"}}"#, server.uri()))
        .await
        .unwrap();

    assert_eq!(fast.content, "independent");
    assert!(slow.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn connection_failure_is_reported_without_a_request() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let tool = fixed_tool_with_timeout(
        "public.test",
        address.port(),
        address,
        Duration::from_secs(1),
    );

    let result = tool
        .execute(&format!(
            r#"{{"url":"http://public.test:{}/"}}"#,
            address.port()
        ))
        .await;

    let error = result.expect_err("closed listener must produce a connection failure");
    assert!(error.to_string().contains("Fetch failed"));
}

#[derive(Debug)]
struct SequencedResolver {
    answers: std::sync::Mutex<std::collections::VecDeque<Vec<SocketAddr>>>,
}

impl DestinationResolver for SequencedResolver {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> ResolutionFuture<'a> {
        let answer = self
            .answers
            .lock()
            .expect("resolver answer lock")
            .pop_front()
            .expect("one deterministic answer per resolution");
        Box::pin(async move { Ok(answer) })
    }
}

#[tokio::test]
async fn redirect_to_denied_hostname_stops_before_dns_or_another_request() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let allowed = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "http://localhost/"))
        .mount(&allowed)
        .await;
    let tool = WebFetchTool::with_allowed_host(32, allowed.uri().trim_start_matches("http://"));

    let result = tool
        .execute(&format!(r#"{{"url":"{}"}}"#, allowed.uri()))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(allowed.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn redirected_host_resolution_is_reauthorized_and_denied_before_send() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let allowed = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/next"))
        .mount(&allowed)
        .await;
    let port = allowed.address().port();
    let resolver = Arc::new(SequencedResolver {
        answers: std::sync::Mutex::new(std::collections::VecDeque::from([
            vec![*allowed.address()],
            vec![SocketAddr::new("127.0.0.2".parse().unwrap(), port)],
        ])),
    });
    let tool = WebFetchTool::with_test_dependencies(
        32,
        loopback_test_policy("public.test", port, allowed.address().ip()),
        resolver,
    );

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(
        allowed.received_requests().await.unwrap().len(),
        1,
        "the denied answer for the redirect hop must never reach a connector"
    );
}

#[tokio::test]
async fn truncated_response_body_is_reported_as_a_read_failure() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let bytes = stream.read(&mut request).await.unwrap();
        assert!(bytes > 0, "server must observe the authorized request");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });
    let tool = fixed_tool_with_timeout(
        "public.test",
        address.port(),
        address,
        Duration::from_secs(1),
    );

    let result = tool
        .execute(&format!(
            r#"{{"url":"http://public.test:{}/"}}"#,
            address.port()
        ))
        .await;

    let error = result.expect_err("premature EOF must fail the body read");
    assert!(error.to_string().contains("Failed to read response body"));
    server.await.unwrap();
}

#[tokio::test]
async fn redirect_whose_hostname_resolves_to_denied_address_sends_zero_requests_there() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let allowed = MockServer::start().await;
    let port = allowed.address().port();
    let denied_listener = std::net::TcpListener::bind(("127.0.0.2", port)).unwrap();
    let denied = MockServer::builder()
        .listener(denied_listener)
        .start()
        .await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("http://redirect.test:{port}/denied")),
        )
        .mount(&allowed)
        .await;

    let resolver = Arc::new(FixedResolver {
        candidates: vec![*allowed.address(), *denied.address()],
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    let policy =
        WebDestinationPolicy::with_test_destination("public.test", port, allowed.address().ip());
    let tool = WebFetchTool::with_test_dependencies(32, policy, resolver);

    let result = tool
        .execute(&format!(r#"{{"url":"http://public.test:{port}/"}}"#))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(allowed.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        denied.received_requests().await.unwrap().len(),
        0,
        "resolved denied redirect candidate must never be sent a request"
    );
}

#[tokio::test]
async fn denied_6to4_resolver_candidate_sends_zero_requests_to_local_server() {
    use wiremock::MockServer;

    let denied = MockServer::start().await;
    let resolver = Arc::new(FixedResolver {
        candidates: vec![SocketAddr::new(
            "2002:7f00:1::1".parse().unwrap(),
            denied.address().port(),
        )],
        calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    });
    let tool = WebFetchTool::with_test_dependencies(32, WebDestinationPolicy::new(), resolver);

    let result = tool
        .execute(&format!(
            r#"{{"url":"http://public.test:{}/"}}"#,
            denied.address().port()
        ))
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(denied.received_requests().await.unwrap().len(), 0);
}

#[tokio::test]
async fn whole_operation_deadline_cancels_a_stalled_response_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1024];
        let bytes = stream.read(&mut request).await.unwrap();
        assert!(bytes > 0, "server must observe the authorized request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: keep-alive\r\n\r\nshort",
            )
            .await
            .unwrap();
        stream.flush().await.unwrap();
        // Keep ownership of the socket across the await so the client sees a
        // genuinely stalled body instead of an early EOF/read failure.
        tokio::time::sleep(Duration::from_secs(2)).await;
        drop(stream);
    });
    let tool = fixed_tool_with_timeout(
        "public.test",
        address.port(),
        address,
        Duration::from_millis(300),
    );
    let started = Instant::now();

    let result = tool
        .execute(&format!(
            r#"{{"url":"http://public.test:{}/"}}"#,
            address.port()
        ))
        .await;

    let error = result.expect_err("stalled body must consume the whole-operation deadline");
    assert!(
        error.to_string().contains("timed out"),
        "unexpected stalled-body error: {error}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
