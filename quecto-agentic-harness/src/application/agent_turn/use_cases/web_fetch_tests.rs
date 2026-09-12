use std::sync::Mutex;

use super::*;

struct RecordingFetch {
    calls: Mutex<Vec<FetchWebContentRequest>>,
    response: Mutex<Option<Result<FetchedWebContent, FetchWebContentError>>>,
}

impl RecordingFetch {
    fn returning(response: Result<FetchedWebContent, FetchWebContentError>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            response: Mutex::new(Some(response)),
        })
    }
}

impl FetchWebContent for RecordingFetch {
    fn fetch(
        &self,
        request: FetchWebContentRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchedWebContent, FetchWebContentError>> + Send + '_>>
    {
        self.calls.lock().unwrap().push(request);
        let response = self.response.lock().unwrap().take().unwrap();
        Box::pin(async move { response })
    }
}

fn fetched(status: u16, body: impl Into<Vec<u8>>) -> FetchedWebContent {
    FetchedWebContent {
        status,
        body: body.into(),
    }
}

async fn execute_fake(
    fake: Arc<RecordingFetch>,
    url: &str,
    raw: bool,
    max_response_kb: u32,
) -> Result<WebFetchResult, WebFetchError> {
    WebFetchUseCase::new(fake, max_response_kb)
        .execute(WebFetchRequest {
            url: url.to_owned(),
            raw,
        })
        .await
}

#[tokio::test]
async fn use_case_invokes_effect_exactly_once_with_owned_url_and_preserves_raw_body() {
    let fake = RecordingFetch::returning(Ok(fetched(200, b"<p>raw</p>".to_vec())));
    let result = execute_fake(fake.clone(), "https://example.com/a", true, 32)
        .await
        .unwrap();

    assert_eq!(result.content, "<p>raw</p>");
    assert!(!result.is_error);
    assert_eq!(
        *fake.calls.lock().unwrap(),
        vec![FetchWebContentRequest {
            url: "https://example.com/a".to_owned()
        }]
    );
}

#[tokio::test]
async fn use_case_owns_readable_transformation() {
    let fake = RecordingFetch::returning(Ok(fetched(
        200,
        b"<header>omit</header><p>Hello&nbsp;<b>world</b></p>".to_vec(),
    )));

    let result = execute_fake(fake, "https://example.com", false, 32)
        .await
        .unwrap();

    assert_eq!(result.content, "Hello world");
    assert!(!result.is_error);
}

#[tokio::test]
async fn use_case_owns_non_success_status_presentation() {
    let fake = RecordingFetch::returning(Ok(fetched(503, b"ignored".to_vec())));

    let result = execute_fake(fake, "https://example.com/down", false, 32)
        .await
        .unwrap();

    assert_eq!(result.content, "HTTP 503 fetching https://example.com/down");
    assert!(result.is_error);
}

#[tokio::test]
async fn use_case_caps_output_on_a_utf8_boundary() {
    let fake = RecordingFetch::returning(Ok(fetched(200, "a".repeat(1023) + "é")));

    let result = execute_fake(fake, "https://example.com", true, 1)
        .await
        .unwrap();

    assert!(result.content.starts_with(&"a".repeat(1023)));
    assert!(!result.content.contains('é'));
    assert!(
        result
            .content
            .ends_with("[Truncated: output exceeded 1KB limit]")
    );
}

#[tokio::test]
async fn use_case_maps_policy_denials_to_established_error_results() {
    let scheme = RecordingFetch::returning(Err(FetchWebContentError::InvalidScheme));
    let denied = RecordingFetch::returning(Err(FetchWebContentError::DestinationDenied(
        "localhost".to_owned(),
    )));

    assert_eq!(
        execute_fake(scheme, "file:///etc/passwd", false, 32)
            .await
            .unwrap(),
        WebFetchResult {
            content:
                "Invalid URL scheme: only http:// and https:// are allowed. Got: file:///etc/passwd"
                    .to_owned(),
            is_error: true,
        }
    );
    assert_eq!(
        execute_fake(denied, "http://localhost", false, 32)
            .await
            .unwrap(),
        WebFetchResult {
            content: "Blocked: URL points to a restricted address (localhost)".to_owned(),
            is_error: true,
        }
    );
}

#[test]
fn fetch_port_is_object_safe_send_and_sync() {
    fn require_port(_: Arc<dyn FetchWebContent + Send + Sync>) {}
    require_port(RecordingFetch::returning(Ok(fetched(200, Vec::new()))));
}

#[test]
fn content_transform_preserves_established_readability_rules() {
    let html = "<HEADER>hidden</HEADER><h1>Title</h1><p>café&nbsp;&amp;&#x42;</p><script>bad()</script>tail<br>line";
    assert_eq!(strip_html(html), "Title\n\ncafé &B\ntail\nline");
    assert_eq!(strip_html("Just plain text."), "Just plain text.");
    assert_eq!(strip_html("<p>A</p>\n\n\n<p>B</p>"), "A\n\nB");
}

#[tokio::test]
async fn use_case_preserves_every_typed_mechanical_error() {
    let cases = [
        (
            FetchWebContentError::InvalidUrl("bad URL".to_owned()),
            WebFetchError::InvalidUrl("bad URL".to_owned()),
        ),
        (
            FetchWebContentError::RedirectLimitExceeded,
            WebFetchError::RedirectLimitExceeded,
        ),
        (FetchWebContentError::TimedOut, WebFetchError::TimedOut),
        (
            FetchWebContentError::ResponseTooLarge {
                actual_bytes: Some(6_000_000),
                max_bytes: 5_242_880,
            },
            WebFetchError::ResponseTooLarge {
                actual_bytes: Some(6_000_000),
                max_bytes: 5_242_880,
            },
        ),
        (
            FetchWebContentError::Resolution("dns".to_owned()),
            WebFetchError::Resolution("dns".to_owned()),
        ),
        (
            FetchWebContentError::Connection("connect".to_owned()),
            WebFetchError::Connection("connect".to_owned()),
        ),
        (
            FetchWebContentError::Read("read".to_owned()),
            WebFetchError::Read("read".to_owned()),
        ),
        (
            FetchWebContentError::Transport("transport".to_owned()),
            WebFetchError::Transport("transport".to_owned()),
        ),
    ];

    for (effect_error, expected) in cases {
        let fake = RecordingFetch::returning(Err(effect_error));
        assert_eq!(
            execute_fake(fake.clone(), "https://example.com", false, 32).await,
            Err(expected)
        );
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }
}

fn target(scheme: &str, host: &str, candidate: Option<IpAddr>) -> WebDestinationTarget {
    WebDestinationTarget::new(scheme, host, 443, candidate)
}

fn authorization(scheme: &str, host: &str) -> WebDestinationAuthorization {
    WebDestinationPolicy::new().authorize(&target(scheme, host, None))
}

#[test]
fn owned_target_constructor_normalizes_and_exposes_adapter_facts() {
    let candidate = "1.1.1.1".parse().unwrap();
    let target = WebDestinationTarget::new("HTTPS", "EXAMPLE.COM", 443, Some(candidate));

    assert_eq!(target.scheme(), "https");
    assert_eq!(target.host(), "example.com");
    assert_eq!(target.effective_port(), Some(443));
    assert_eq!(target.candidate(), Some(candidate));
}

#[test]
fn scheme_allowlist_contains_exactly_http_and_https() {
    for scheme in ["http", "https", "HTTP", "Https"] {
        assert_eq!(
            authorization(scheme, "example.com"),
            WebDestinationAuthorization::Allowed,
            "expected normalized {scheme:?} to be allowed"
        );
    }
    for scheme in ["", "ftp", "file", "data", "javascript", "http ", "https+"] {
        assert_eq!(
            authorization(scheme, "example.com"),
            WebDestinationAuthorization::Denied,
            "expected {scheme:?} to be denied"
        );
    }
}

#[test]
fn public_literal_destinations_are_allowed() {
    for host in [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "[2606:4700:4700::1111]",
        "2606:4700:4700::1111",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Allowed,
            "expected {host} to be public"
        );
    }
}

#[test]
fn every_non_public_ipv4_class_is_denied() {
    for host in [
        "0.0.0.0",         // unspecified / this network
        "0.1.2.3",         // this network
        "10.0.0.1",        // private
        "100.64.0.1",      // shared address space
        "127.0.0.1",       // loopback
        "127.255.255.254", // loopback range boundary
        "169.254.169.254", // link-local / metadata
        "172.16.0.1",      // private
        "172.31.255.255",  // private boundary
        "192.0.0.1",       // IETF protocol assignments
        "192.0.2.1",       // documentation
        "192.88.99.1",     // deprecated relay anycast
        "192.168.1.1",     // private
        "198.18.0.1",      // benchmarking
        "198.51.100.1",    // documentation
        "203.0.113.1",     // documentation
        "224.0.0.1",       // multicast
        "240.0.0.1",       // reserved
        "255.255.255.255", // broadcast
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host} to be denied"
        );
    }
}

#[test]
fn every_non_public_ipv6_class_is_denied() {
    for host in [
        "::",                 // unspecified
        "::1",                // loopback
        "::ffff:127.0.0.1",   // IPv4-mapped loopback
        "64:ff9b::192.0.2.1", // translation prefix
        "100::1",             // discard-only
        "2001:db8::1",        // documentation
        "3fff::1",            // documentation
        "fc00::1",            // unique-local
        "fd12:3456::1",       // unique-local
        "fe80::1",            // link-local
        "ff02::1",            // multicast
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host} to be denied"
        );
    }
}

#[test]
fn known_restricted_domain_names_are_denied_after_normalization() {
    for host in [
        "localhost",
        "LOCALHOST",
        "localhost.",
        "metadata.google.internal",
        "METADATA.GOOGLE.INTERNAL.",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Denied,
            "expected {host:?} to be denied"
        );
    }
}

#[test]
fn syntactically_public_dns_names_are_allowed_pending_candidate_authorization() {
    for host in [
        "example.com",
        "api.example.com",
        "xn--bcher-kva.example",
        "example.com.",
    ] {
        assert_eq!(
            authorization("https", host),
            WebDestinationAuthorization::Allowed,
            "expected {host:?} to be provisionally allowed"
        );
    }
}

#[test]
fn malformed_or_unclassified_facts_fail_closed() {
    let policy = WebDestinationPolicy::new();
    let cases = [
        WebDestinationTarget::new("https", "", 443, None),
        WebDestinationTarget::new("https", "single-label", 443, None),
        WebDestinationTarget::new("https", ".example.com", 443, None),
        WebDestinationTarget::new("https", "example..com", 443, None),
        WebDestinationTarget::new("https", "-bad.example", 443, None),
        WebDestinationTarget::new("https", "bad-.example", 443, None),
        WebDestinationTarget::new("https", "bad_name.example", 443, None),
        WebDestinationTarget::new("https", "999.999.999.999", 443, None),
        WebDestinationTarget::new("https", "[not-ipv6]", 443, None),
        WebDestinationTarget::new("https", "example.com", None, None),
        WebDestinationTarget::new("https", "example.com", 0, None),
    ];

    for malformed in cases {
        assert_eq!(
            policy.authorize(&malformed),
            WebDestinationAuthorization::Denied,
            "expected malformed target {malformed:?} to fail closed"
        );
    }
}

#[test]
fn supplied_candidate_requires_its_own_affirmative_authorization() {
    let policy = WebDestinationPolicy::new();
    let public = target(
        "https",
        "public-looking.example",
        Some("1.1.1.1".parse().unwrap()),
    );
    let private = target(
        "https",
        "public-looking.example",
        Some("10.0.0.7".parse().unwrap()),
    );
    let loopback = target(
        "https",
        "public-looking.example",
        Some("::1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&public),
        WebDestinationAuthorization::Allowed
    );
    assert_eq!(
        policy.authorize(&private),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&loopback),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn exact_test_destination_allows_only_that_host_and_port() {
    let policy = WebDestinationPolicy::with_test_destination("allowed.test", 8443);
    let exact = WebDestinationTarget::new(
        "http",
        "ALLOWED.TEST",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_host = WebDestinationTarget::new(
        "http",
        "other.test",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_port = WebDestinationTarget::new(
        "http",
        "allowed.test",
        8444,
        Some("127.0.0.1".parse().unwrap()),
    );
    let wrong_scheme = WebDestinationTarget::new(
        "ftp",
        "allowed.test",
        8443,
        Some("127.0.0.1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&exact),
        WebDestinationAuthorization::Allowed
    );
    assert_eq!(
        policy.authorize(&wrong_host),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&wrong_port),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        policy.authorize(&wrong_scheme),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn malformed_test_permission_cannot_bypass_fail_closed_validation() {
    let empty_host = WebDestinationPolicy::with_test_destination("", 8443);
    let zero_port = WebDestinationPolicy::with_test_destination("localhost", 0);

    assert_eq!(
        empty_host.authorize(&WebDestinationTarget::new("http", "", 8443, None)),
        WebDestinationAuthorization::Denied
    );
    assert_eq!(
        zero_port.authorize(&WebDestinationTarget::new("http", "localhost", 0, None)),
        WebDestinationAuthorization::Denied
    );
}

#[test]
fn exact_literal_test_destination_may_reach_its_matching_loopback_server() {
    let policy = WebDestinationPolicy::with_test_destination("127.0.0.1", 32123);
    let target = WebDestinationTarget::new(
        "http",
        "127.0.0.1",
        32123,
        Some("127.0.0.1".parse().unwrap()),
    );

    assert_eq!(
        policy.authorize(&target),
        WebDestinationAuthorization::Allowed
    );
}
