use crate::{
    application::agent_turn::use_cases::web_fetch::{FetchWebContent, WebFetchUseCase},
    application::tools::ports::Tool,
    infrastructure::http::web_fetch::ReqwestFetchWebContent,
    interface::tools::web_fetch::WebFetchTool,
};
use std::sync::Arc;
/// The web-fetch graph over the shared client recipe: its headers, TLS trust
/// and timeouts are kept, and the adapter lays destination enforcement over
/// them (#1942).
pub fn build(client: reqwest::ClientBuilder, max_response_kb: u32) -> Arc<dyn Tool> {
    graph(ReqwestFetchWebContent::new(client), max_response_kb)
}
/// [`build`] for tests whose local servers listen on 127.0.0.1.
#[cfg(any(test, feature = "test-support"))]
pub fn build_allowing_loopback_for_tests(
    client: reqwest::ClientBuilder,
    max_response_kb: u32,
) -> Arc<dyn Tool> {
    graph(
        ReqwestFetchWebContent::allowing_loopback_for_tests(client),
        max_response_kb,
    )
}
fn graph(adapter: ReqwestFetchWebContent, max_response_kb: u32) -> Arc<dyn Tool> {
    let adapter: Arc<dyn FetchWebContent> = Arc::new(adapter);
    Arc::new(WebFetchTool::new(Arc::new(WebFetchUseCase::new(
        adapter,
        max_response_kb,
    ))))
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
