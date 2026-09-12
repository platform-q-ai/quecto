use quecto::application::agent_turn::use_cases::web_fetch::{
    FetchWebContent, FetchWebContentError, FetchWebContentRequest, FetchedWebContent,
};
use std::sync::Arc;

struct ContractAdapter;
impl FetchWebContent for ContractAdapter {
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

#[tokio::test]
async fn fetch_web_content_port_is_object_safe_and_preserves_owned_contract_values() {
    let port: Arc<dyn FetchWebContent + Send + Sync> = Arc::new(ContractAdapter);
    let result = port
        .fetch(FetchWebContentRequest {
            url: "https://example.com".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        result,
        FetchedWebContent {
            status: 200,
            body: b"https://example.com".to_vec()
        }
    );
}
