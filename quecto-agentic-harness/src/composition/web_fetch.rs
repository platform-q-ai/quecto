use crate::{
    application::agent_turn::use_cases::web_fetch::{FetchWebContent, WebFetchUseCase},
    application::tools::ports::Tool,
    infrastructure::http::web_fetch::{ReqwestFetchWebContent, WebFetchClientRecipe},
    interface::tools::web_fetch::WebFetchTool,
};
use std::sync::Arc;
/// The web-fetch graph over its client recipe (the settings production
/// chooses, [`crate::interface::shared::web_fetch_client_recipe`]): the
/// adapter builds its client from it and lays destination enforcement over
/// it (#1942).
pub fn build(recipe: WebFetchClientRecipe, max_response_kb: u32) -> Arc<dyn Tool> {
    graph(ReqwestFetchWebContent::new(recipe), max_response_kb)
}
/// [`build`] for tests whose local servers listen on 127.0.0.1.
#[cfg(any(test, feature = "test-support"))]
pub fn build_allowing_loopback_for_tests(
    recipe: WebFetchClientRecipe,
    max_response_kb: u32,
) -> Arc<dyn Tool> {
    graph(
        ReqwestFetchWebContent::allowing_loopback_for_tests(recipe),
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
