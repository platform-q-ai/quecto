#[cfg(any(test, feature = "test-support"))]
pub fn built_tool_result_preview_count_for_tests(content: &str) -> usize {
    super::agent_loop::agent_loop_preview::built_preview_count_for_tests(content)
}

pub fn reset_built_tool_result_preview_count_for_tests(content: &str) {
    super::agent_loop::agent_loop_preview::reset_built_preview_count_for_tests(content);
}

pub use crate::domain::message::{
    reset_tool_call_clone_count_for_tests, tool_call_clone_count_for_tests,
};
