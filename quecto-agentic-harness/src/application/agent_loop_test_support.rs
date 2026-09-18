#[cfg(any(test, feature = "test-support"))]
pub fn built_tool_result_preview_count_for_tests(content: &str) -> usize {
    super::agent_loop::agent_loop_preview::built_preview_count_for_tests(content)
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_built_tool_result_preview_count_for_tests(content: &str) {
    super::agent_loop::agent_loop_preview::reset_built_preview_count_for_tests(content);
}

pub use crate::domain::message::{
    reset_tool_call_clone_count_for_tests, tool_call_clone_count_for_tests,
};

/// A registered tool run through the loop's own registry, for tests that
/// reach a tool through the real agent build (`build_agent_from_config`)
/// rather than a hand-composed fixture (#2024 S4a review).
#[cfg(any(test, feature = "test-support"))]
impl super::agent_loop::AgentLoopImpl {
    pub fn execute_tool_for_tests(
        &self,
        name: &str,
        arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::domain::tool::ToolResult,
                        crate::domain::error::DomainError,
                    >,
                > + Send
                + '_,
        >,
    > {
        self.tool_registry.execute(name, arguments)
    }
}
