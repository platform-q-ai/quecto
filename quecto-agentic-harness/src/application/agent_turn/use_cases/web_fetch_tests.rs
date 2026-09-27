use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
struct Fake {
    calls: AtomicUsize,
    outcome: FetchOutcome,
}
impl FetchWebContent for Fake {
    fn fetch<'a>(
        &'a self,
        _: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let out = self.outcome.clone();
        Box::pin(async move { Ok(out) })
    }
}
fn use_case(outcome: FetchOutcome) -> (Arc<Fake>, WebFetchUseCase) {
    let f = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        outcome,
    });
    (f.clone(), WebFetchUseCase::new(f, 32))
}
#[tokio::test]
async fn rejection_categories_never_call_port() {
    for (url, expected) in [
        ("not a url", "error"),
        ("ftp://example.com", "scheme"),
        ("http://localhost", "host"),
        ("http://10.0.0.1", "host"),
        ("http://[::1]", "host"),
    ] {
        let (f, u) = use_case(FetchOutcome::SuccessBody {
            body: vec![],
            content_type: None,
        });
        let got = u.execute(url, HtmlView::MainContent).await;
        match expected {
            "error" => assert!(got.is_err()),
            "scheme" => assert_eq!(got.unwrap(), WebFetchResult::UnsupportedScheme),
            "host" => assert!(matches!(
                got.unwrap(),
                WebFetchResult::RestrictedInitialHost(_)
            )),
            _ => unreachable!(),
        }
        assert_eq!(f.calls.load(Ordering::SeqCst), 0, "{url}");
    }
}
#[tokio::test]
async fn accepted_baseline_hosts_call_port_once() {
    for url in [
        "https://example.com",
        "http://8.8.8.8",
        "http://[2606:4700:4700::1111]",
        "http://[::ffff:8.8.8.8]",
        // Only whole labels: these merely contain a local name.
        "http://notlocalhost/",
        "http://localhost.example.com/",
        "http://metadata.example.com/",
        "http://goog.metadata.example/",
    ] {
        let (f, u) = use_case(FetchOutcome::SuccessBody {
            body: b"ok".to_vec(),
            content_type: None,
        });
        assert_eq!(
            u.execute(url, HtmlView::Markup).await.unwrap(),
            WebFetchResult::Success("ok".into())
        );
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
}
/// #1942: every spelling of a non-public address is refused before the
/// port is called, naming the address it reaches.
#[tokio::test]
async fn non_public_address_literals_in_any_spelling_never_call_port() {
    for (url, reason) in [
        (
            "http://[::ffff:127.0.0.1]:1/",
            "127.0.0.1 (via ::ffff:127.0.0.1) is not a public address",
        ),
        (
            "http://[::ffff:a9fe:a9fe]/",
            "169.254.169.254 (via ::ffff:169.254.169.254) is not a public address",
        ),
        (
            "http://[64:ff9b::a9fe:a9fe]/",
            "169.254.169.254 (via 64:ff9b::a9fe:a9fe) is not a public address",
        ),
        (
            "http://[2002:a9fe:a9fe::1]/",
            "169.254.169.254 (via 2002:a9fe:a9fe::1) is not a public address",
        ),
        ("http://[fd00::1]/", "fd00::1 is not a public address"),
        ("http://[fe80::1]/", "fe80::1 is not a public address"),
        (
            "http://[2001:db8::1]/",
            "2001:db8::1 is not a public address",
        ),
        ("http://100.64.0.1/", "100.64.0.1 is not a public address"),
        ("http://198.18.0.1/", "198.18.0.1 is not a public address"),
        ("http://0x7f.1/", "127.0.0.1 is not a public address"),
        (
            "http://2852039166/",
            "169.254.169.254 is not a public address",
        ),
        ("http://127.1/", "127.0.0.1 is not a public address"),
        ("http://localhost./", "localhost. is a local name"),
        ("http://LOCALHOST/", "localhost is a local name"),
        ("http://foo.localhost/", "foo.localhost is a local name"),
        ("http://a.b.localhost./", "a.b.localhost. is a local name"),
        ("http://metadata/", "metadata is a local name"),
        ("http://metadata.goog./", "metadata.goog. is a local name"),
        (
            "http://metadata.google.internal/",
            "metadata.google.internal is a local name",
        ),
    ] {
        let (f, u) = use_case(FetchOutcome::SuccessBody {
            body: vec![],
            content_type: None,
        });
        assert_eq!(
            u.execute(url, false).await.unwrap(),
            WebFetchResult::RestrictedInitialHost(reason.into()),
            "{url}"
        );
        assert_eq!(f.calls.load(Ordering::SeqCst), 0, "{url}");
    }
}
#[tokio::test]
async fn non_success_is_preserved_without_body_shape() {
    let (_, u) = use_case(FetchOutcome::NonSuccessStatus(HttpStatus::new(
        404,
        Some("Not Found".into()),
    )));
    assert_eq!(
        u.execute("https://example.com", HtmlView::MainContent)
            .await
            .unwrap(),
        WebFetchResult::NonSuccessStatus(HttpStatus::new(404, Some("Not Found".into())))
    );
}
#[test]
fn readable_and_utf8_helpers_remain_owned_here() {
    assert_eq!(strip_html("<p>Hello&nbsp;world</p>"), "Hello world");
    assert!(truncate_output("éé".into(), 0).starts_with("\n\n[Truncated"));
}
