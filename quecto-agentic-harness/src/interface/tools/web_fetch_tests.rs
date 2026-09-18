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
