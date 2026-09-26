//! #2165: web_fetch answers by the content it got: HTML is made readable,
//! other text comes back as it is, and binary content is named, not shown.
use super::*;

struct Fixed(FetchOutcome);
impl FetchWebContent for Fixed {
    fn fetch<'a>(
        &'a self,
        _: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        let out = self.0.clone();
        Box::pin(async move { Ok(out) })
    }
}

async fn fetched(body: &[u8], content_type: Option<&str>, raw: bool) -> WebFetchResult {
    let fetcher = Arc::new(Fixed(FetchOutcome::SuccessBody {
        body: body.to_vec(),
        content_type: content_type.map(str::to_owned),
    }));
    WebFetchUseCase::new(fetcher, 32)
        .execute("https://example.com/x", raw)
        .await
        .unwrap()
}

#[tokio::test]
async fn binary_content_is_named_not_shown() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(2048).collect();
    for content_type in [Some("application/octet-stream"), Some("image/png"), None] {
        let got = fetched(&bytes, content_type, false).await;
        assert_eq!(
            got,
            WebFetchResult::Binary {
                content_type: content_type.map(str::to_owned),
                bytes: 2048
            },
            "{content_type:?}"
        );
    }
}

#[tokio::test]
async fn text_that_is_not_html_comes_back_as_it_is() {
    let json = br#"{"a": "<b> & <c>"}"#;
    for content_type in [
        "application/json",
        "text/plain; charset=utf-8",
        "application/problem+json",
        "text/markdown",
    ] {
        let got = fetched(json, Some(content_type), false).await;
        assert_eq!(
            got,
            WebFetchResult::Success(String::from_utf8(json.to_vec()).unwrap()),
            "{content_type}"
        );
    }
}

#[tokio::test]
async fn html_is_made_readable_and_raw_keeps_it() {
    let html = b"<html><body><p>Hello &amp; welcome</p></body></html>";
    let got = fetched(html, Some("text/html; charset=UTF-8"), false).await;
    assert_eq!(got, WebFetchResult::Success("Hello & welcome".into()));
    let raw = fetched(html, Some("text/html"), true).await;
    assert_eq!(
        raw,
        WebFetchResult::Success(String::from_utf8(html.to_vec()).unwrap())
    );
}

/// Without a content type the bytes decide: HTML by its opening, other
/// valid text as it is.
#[tokio::test]
async fn untyped_content_is_sniffed() {
    let got = fetched(b"  <!DOCTYPE html><p>hi</p>", None, false).await;
    assert_eq!(got, WebFetchResult::Success("hi".into()));
    let got = fetched(b"plain <words>", None, false).await;
    assert_eq!(got, WebFetchResult::Success("plain <words>".into()));
}

/// An unclosed stripped block drops only its opening tag, never the rest
/// of the page.
#[test]
fn an_unclosed_block_keeps_the_rest_of_the_page() {
    let text = strip_html("<p>before</p><nav><p>after</p><p>end</p>");
    assert!(text.contains("before"), "{text}");
    assert!(text.contains("after"), "{text}");
    assert!(text.contains("end"), "{text}");
}

/// Untyped bytes that decode as UTF-8 but hold a NUL are binary.
#[tokio::test]
async fn untyped_bytes_with_a_nul_are_binary() {
    let got = fetched(b"PK\x03\x04\x00\x00data", None, false).await;
    assert_eq!(
        got,
        WebFetchResult::Binary {
            content_type: None,
            bytes: 10
        }
    );
}
