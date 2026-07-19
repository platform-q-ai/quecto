#[test]
fn public_agent_loop_test_support_forwards_preview_helpers() {
    let content = "preview helper public wrapper coverage";
    crate::application::agent_loop_test_support::reset_built_tool_result_preview_count_for_tests(
        content,
    );
    assert_eq!(
        crate::application::agent_loop_test_support::built_tool_result_preview_count_for_tests(
            content
        ),
        0
    );
    let _ = super::agent_loop_preview::tool_result_preview(content);
    assert_eq!(
        crate::application::agent_loop_test_support::built_tool_result_preview_count_for_tests(
            content
        ),
        1
    );
}
