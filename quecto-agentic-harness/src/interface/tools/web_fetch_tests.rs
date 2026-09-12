use super::*;
use crate::application::agent_turn::use_cases::web_fetch::{FetchOutcome, FetchRequest, FetchWebContent, HttpStatus};

struct StatusFetcher(HttpStatus);
impl FetchWebContent for StatusFetcher {
    fn fetch<'a>(&'a self, _: &'a FetchRequest) -> Pin<Box<dyn Future<Output = Result<FetchOutcome, FetchFailure>> + Send + 'a>> {
        Box::pin(async { Ok(FetchOutcome::NonSuccessStatus(self.0.clone())) })
    }
}

async fn content(status: HttpStatus) -> String {
    let fetcher: Arc<dyn FetchWebContent> = Arc::new(StatusFetcher(status));
    let tool = WebFetchTool::new(Arc::new(WebFetchUseCase::new(fetcher, 32)));
    tool.execute(r#"{"url":"https://example.com"}"#).await.unwrap().content
}

#[tokio::test]
async fn non_success_status_preserves_baseline_display_for_known_and_unknown_codes() {
    assert_eq!(content(HttpStatus::new(404, Some("Not Found".into()))).await,
        "HTTP 404 Not Found fetching https://example.com");
    assert_eq!(content(HttpStatus::new(599, None)).await,
        "HTTP 599 fetching https://example.com");
}
