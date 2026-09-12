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
        let (f, u) = use_case(FetchOutcome::SuccessBody(vec![]));
        let got = u.execute(url, false).await;
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
        "http://[2001:db8::1]",
    ] {
        let (f, u) = use_case(FetchOutcome::SuccessBody(b"ok".to_vec()));
        assert_eq!(
            u.execute(url, true).await.unwrap(),
            WebFetchResult::Success("ok".into())
        );
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn non_success_is_preserved_without_body_shape() {
    let (_, u) = use_case(FetchOutcome::NonSuccessStatus(HttpStatus::new(
        404,
        Some("Not Found".into()),
    )));
    assert_eq!(
        u.execute("https://example.com", false).await.unwrap(),
        WebFetchResult::NonSuccessStatus(HttpStatus::new(404, Some("Not Found".into())))
    );
}
#[test]
fn readable_and_utf8_helpers_remain_owned_here() {
    assert_eq!(strip_html("<p>Hello&nbsp;world</p>"), "Hello world");
    assert!(truncate_output("éé".into(), 0).starts_with("\n\n[Truncated"));
}
