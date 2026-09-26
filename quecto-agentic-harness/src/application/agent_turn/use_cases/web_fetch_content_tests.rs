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

/// #2177 review: a page of many unclosed blocks is stripped in linear time.
#[test]
fn many_unclosed_blocks_strip_in_linear_time() {
    let page = "<nav>x".repeat(160_000);
    let started = std::time::Instant::now();
    let text = strip_html(&page);
    assert!(text.contains('x'));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
}

/// #2177 review: text served as octet-stream, or with an empty or
/// malformed type, is sniffed rather than refused.
#[tokio::test]
async fn octet_stream_and_bad_types_are_sniffed() {
    for content_type in ["application/octet-stream", "", "text"] {
        let got = fetched(b"# notes\nplain text", Some(content_type), false).await;
        assert_eq!(
            got,
            WebFetchResult::Success("# notes\nplain text".into()),
            "{content_type:?}"
        );
    }
}

/// #2177 review: line-delimited JSON and CSV are text.
#[tokio::test]
async fn ndjson_jsonl_and_csv_are_text() {
    for content_type in [
        "application/x-ndjson",
        "application/jsonl",
        "application/jsonlines",
        "application/csv",
    ] {
        let got = fetched(b"{\"a\":1}\n", Some(content_type), false).await;
        assert_eq!(
            got,
            WebFetchResult::Success("{\"a\":1}\n".into()),
            "{content_type}"
        );
    }
}

/// #2177 review: untyped HTML is recognised after a BOM, an XML prolog or a
/// leading comment.
#[tokio::test]
async fn untyped_html_after_a_bom_prolog_or_comment_is_html() {
    for page in [
        "\u{feff}<!DOCTYPE html><p>hi</p>",
        "<?xml version=\"1.0\"?><!DOCTYPE html><p>hi</p>",
        "<!-- built --><html><p>hi</p></html>",
    ] {
        let got = fetched(page.as_bytes(), None, false).await;
        assert_eq!(got, WebFetchResult::Success("hi".into()), "{page}");
    }
}

/// #2177 review 2: stray `<` with no `>`, and `&` with a far `;`, strip in
/// linear time.
#[test]
fn stray_brackets_and_ampersands_strip_in_linear_time() {
    for page in ["<".repeat(400_000), format!("{};", "&".repeat(400_000))] {
        let started = std::time::Instant::now();
        let _ = strip_html(&page);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }
}

/// #2177 review 2: S3's label for untyped uploads is sniffed too.
#[tokio::test]
async fn binary_octet_stream_and_unknown_are_sniffed() {
    for content_type in ["binary/octet-stream", "application/unknown"] {
        let got = fetched(b"# notes", Some(content_type), false).await;
        assert_eq!(
            got,
            WebFetchResult::Success("# notes".into()),
            "{content_type}"
        );
    }
}
