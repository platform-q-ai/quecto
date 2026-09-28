use super::*;
use futures::FutureExt;

#[tokio::test]
async fn the_tool_place_panics_inside_the_call() {
    let tool = PanicProbeTool;
    let outcome = std::panic::AssertUnwindSafe(tool.execute(r#"{"where":"tool"}"#))
        .catch_unwind()
        .await;
    let payload = outcome.expect_err("the probe panics");
    let message = crate::application::tool_panic_scope::payload_message(payload.as_ref());
    assert!(message.contains("is not a char boundary"), "{message}");
}

#[tokio::test]
async fn an_unknown_place_is_an_error_not_a_panic() {
    let outcome = PanicProbeTool.execute(r#"{"where":"nowhere"}"#).await;
    let error = outcome.expect_err("refused").to_string();
    assert!(error.contains("must be \"tool\" or \"outside\""), "{error}");
}

#[tokio::test]
async fn without_the_process_hook_an_outside_panic_only_ends_its_thread() {
    // The fatal abort is the process hook's (see `panic_hook` tests); here,
    // with no hook installed, the probe's thread dies alone.
    let result = PanicProbeTool
        .execute(r#"{"where":"outside"}"#)
        .await
        .unwrap();
    assert_eq!(result.content, "the process survived a fatal panic");
}

#[test]
fn the_probe_is_named_for_the_tests_that_drive_it() {
    assert_eq!(PanicProbeTool.definition().name, PANIC_PROBE_TOOL);
}
