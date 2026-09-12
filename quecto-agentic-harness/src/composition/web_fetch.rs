//! The single concrete graph for the web-fetch capability.
use std::sync::Arc;

use crate::application::agent_turn::use_cases::web_fetch::WebFetchUseCase;
use crate::domain::tool::Tool;
use crate::infrastructure::tools::web_fetch::ReqwestFetchWebContent;
use crate::interface::tools::web_fetch::WebFetchTool;

pub fn build_web_fetch_tool(max_response_kb: u32) -> Arc<dyn Tool> {
    build_web_fetch_tool_with_content(Arc::new(ReqwestFetchWebContent::new()), max_response_kb)
}

#[cfg(any(test, feature = "test-support"))]
pub fn build_web_fetch_tool_for_destination(
    max_response_kb: u32,
    host_port: &str,
) -> Arc<dyn Tool> {
    build_web_fetch_tool_with_content(
        Arc::new(ReqwestFetchWebContent::with_allowed_host(host_port)),
        max_response_kb,
    )
}

fn build_web_fetch_tool_with_content(
    content: Arc<dyn crate::application::agent_turn::use_cases::web_fetch::FetchWebContent>,
    max_response_kb: u32,
) -> Arc<dyn Tool> {
    Arc::new(WebFetchTool::new(WebFetchUseCase::new(
        content,
        max_response_kb,
    )))
}
