use super::*;
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchOutcome, FetchRequest, FetchWebContent, HttpStatus,
};

struct StatusFetcher(HttpStatus);
impl FetchWebContent for StatusFetcher {
    fn fetch<'a>(
        &'a self,
        _: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        Box::pin(async { Ok(FetchOutcome::NonSuccessStatus(self.0.clone())) })
    }
}

async fn content(status: HttpStatus) -> String {
    let fetcher: Arc<dyn FetchWebContent> = Arc::new(StatusFetcher(status));
    let tool = WebFetchTool::new(Arc::new(WebFetchUseCase::new(fetcher, 32)));
    tool.execute(r#"{"url":"https://example.com"}"#)
        .await
        .unwrap()
        .content
}

#[tokio::test]
async fn non_success_status_preserves_baseline_display_for_known_and_unknown_codes() {
    assert_eq!(
        content(HttpStatus::new(404, Some("Not Found".into()))).await,
        "HTTP 404 Not Found fetching https://example.com"
    );
    assert_eq!(
        content(HttpStatus::new(599, None)).await,
        "HTTP 599 fetching https://example.com"
    );
}

#[test]
fn every_fetch_failure_becomes_a_tool_error_naming_what_went_wrong() {
    use crate::application::agent_turn::use_cases::web_fetch::FetchFailure;
    use crate::domain::error::DomainError;
    let message = |failure: FetchFailure| match super::map_failure(failure, "https://x.test/") {
        DomainError::Tool(message) => message,
        other => panic!("expected a tool error, got {other:?}"),
    };
    assert_eq!(
        message(FetchFailure::TimedOut),
        "Request timed out after 10s: https://x.test/"
    );
    assert_eq!(
        message(FetchFailure::TooLarge {
            actual_bytes: Some(2048),
            max_bytes: 1024
        }),
        "Response too large: 2048 bytes (max 1024)"
    );
    assert_eq!(
        message(FetchFailure::TooLarge {
            actual_bytes: None,
            max_bytes: 1024
        }),
        "Response too large: >1024 bytes (max 1024)"
    );
    assert_eq!(
        message(FetchFailure::Read("eof".into())),
        "Failed to read response body: eof"
    );
    assert_eq!(
        message(FetchFailure::Transport("dns".into())),
        "Fetch failed: dns"
    );
}

/// #2165: binary content is named, with its type and size, and is not an
/// error (the fetch worked).
#[tokio::test]
async fn binary_content_is_named_not_shown() {
    struct Binary;
    impl FetchWebContent for Binary {
        fn fetch<'a>(
            &'a self,
            _: &'a FetchRequest,
        ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
            Box::pin(async {
                Ok(FetchOutcome::SuccessBody {
                    body: vec![0xff; 2048],
                    content_type: Some("image/png".into()),
                })
            })
        }
    }
    let tool = WebFetchTool::new(Arc::new(WebFetchUseCase::new(Arc::new(Binary), 32)));
    let result = tool
        .execute(r#"{"url":"https://example.com/a.png"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(
        result.content,
        "https://example.com/a.png is binary content (image/png, 2048 bytes), not shown: web_fetch returns text and HTML only"
    );
}

struct HtmlFetcher(&'static str);
impl FetchWebContent for HtmlFetcher {
    fn fetch<'a>(
        &'a self,
        _: &'a FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        let body = self.0.as_bytes().to_vec();
        Box::pin(async move {
            Ok(FetchOutcome::SuccessBody {
                body,
                content_type: Some("text/html".into()),
            })
        })
    }
}

/// #2165 review: HTML reads its main content by default; `main_only:
/// false` reads the whole page; `raw` still wins over both. The schema and
/// the description name the option.
#[tokio::test]
async fn main_only_chooses_between_the_main_content_and_the_whole_page() {
    let page = "<html><head><title>T</title></head><body><p>side side side side side side</p><main><p>in in in in in in in in in in in in in in in in in in in in</p></main></body></html>";
    let fetcher: Arc<dyn FetchWebContent> = Arc::new(HtmlFetcher(page));
    let tool = WebFetchTool::new(Arc::new(WebFetchUseCase::new(fetcher, 32)));
    let read = |args: &'static str| {
        let tool = &tool;
        async move { tool.execute(args).await.unwrap().content }
    };
    let main = read(r#"{"url":"https://example.com"}"#).await;
    assert!(
        main.contains("main_only: false") && !main.contains("side"),
        "{main}"
    );
    let whole = read(r#"{"url":"https://example.com","main_only":false}"#).await;
    assert!(
        whole.contains("side") && !whole.contains("main_only"),
        "{whole}"
    );
    let raw = read(r#"{"url":"https://example.com","raw":true,"main_only":true}"#).await;
    assert_eq!(raw, page);
    let definition = tool.definition();
    assert!(definition.parameters_schema.contains("\"main_only\""));
    assert!(definition.description.contains("main_only"));
}
