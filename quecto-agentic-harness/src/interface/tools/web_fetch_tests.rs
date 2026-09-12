use super::*;
use crate::application::agent_turn::use_cases::web_fetch::{
    FetchWebContent, FetchWebContentError, FetchWebContentRequest, FetchedWebContent,
};
use std::sync::Arc;

struct Fake;
impl FetchWebContent for Fake {
    fn fetch(
        &self,
        request: FetchWebContentRequest,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<FetchedWebContent, FetchWebContentError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Ok(FetchedWebContent {
                status: 200,
                body: request.url.into_bytes(),
            })
        })
    }
}
fn tool() -> WebFetchTool {
    WebFetchTool::new(WebFetchUseCase::new(Arc::new(Fake), 32))
}

#[tokio::test]
async fn json_adapter_decodes_request_and_defaults_raw_false() {
    let result = tool()
        .execute(r#"{"url":"https://example.com/<b>x</b>"}"#)
        .await
        .unwrap();
    assert_eq!(result.content, "https://example.com/x");
    assert!(!result.is_error);
}
#[tokio::test]
async fn json_adapter_rejects_invalid_and_missing_url() {
    assert!(
        tool()
            .execute("{")
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid JSON")
    );
    assert!(
        tool()
            .execute("{}")
            .await
            .unwrap_err()
            .to_string()
            .contains("missing required field")
    );
}
#[test]
fn schema_and_identity_remain_stable() {
    let definition = tool().definition();
    assert_eq!(definition.name.as_ref(), "web_fetch");
    assert!(definition.parameters_schema.contains("raw"));
}
