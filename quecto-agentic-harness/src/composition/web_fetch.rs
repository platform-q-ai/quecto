use crate::{
    application::agent_turn::use_cases::web_fetch::{FetchWebContent, WebFetchUseCase},
    domain::tool::Tool,
    infrastructure::http::web_fetch::ReqwestFetchWebContent,
    interface::tools::web_fetch::WebFetchTool,
};
use std::sync::Arc;
pub fn build(client: reqwest::Client, max_response_kb: u32) -> Arc<dyn Tool> {
    let adapter: Arc<dyn FetchWebContent> = Arc::new(ReqwestFetchWebContent::new(client));
    Arc::new(WebFetchTool::new(Arc::new(WebFetchUseCase::new(
        adapter,
        max_response_kb,
    ))))
}
