use super::*;

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
) -> (ReqwestFetchWebContent, Arc<std::sync::atomic::AtomicUsize>) {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let resolver = Arc::new(FixedResolver {
        candidates,
        calls: Arc::clone(&calls),
    });
    let policy = WebDestinationPolicy::with_test_destination(host, port);
    (
        ReqwestFetchWebContent::with_test_dependencies(policy, resolver),
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
        .fetch(FetchWebContentRequest {
            url: format!("http://public.test:{port}/"),
        })
        .await
        .unwrap();

    assert_eq!(result.body, b"pinned");
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
    let tool =
        ReqwestFetchWebContent::with_test_dependencies(WebDestinationPolicy::new(), resolver);

    let result = tool
        .fetch(FetchWebContentRequest {
            url: format!("http://public.test:{port}/"),
        })
        .await
        .unwrap_err();

    assert!(matches!(result, FetchWebContentError::DestinationDenied(_)));
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
        .fetch(FetchWebContentRequest {
            url: format!("http://public.test:{port}/"),
        })
        .await
        .unwrap_err();

    assert!(matches!(result, FetchWebContentError::DestinationDenied(_)));
    assert_eq!(allowed.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        denied.received_requests().await.unwrap().len(),
        0,
        "redirect denial must happen before send"
    );
}
