use super::*;
use crate::application::tool_panic_scope::{ToolScope, current, scoped};
use futures::FutureExt;

fn tool_now() -> Option<String> {
    current().map(|scope| scope.tool().to_string())
}

#[tokio::test]
async fn carried_blocking_work_runs_in_the_call_and_returns_its_value() {
    let seen = scoped(ToolScope::new("grep"), async {
        spawn_blocking_in_call(tool_now).await.unwrap()
    })
    .await;
    assert_eq!(seen.as_deref(), Some("grep"));
}

#[tokio::test]
async fn a_carried_task_runs_in_the_call() {
    let seen = scoped(ToolScope::new("grep"), async {
        spawn_in_call(async { tool_now() }).await.unwrap()
    })
    .await;
    assert_eq!(seen.as_deref(), Some("grep"));
}

#[tokio::test]
async fn a_panic_in_carried_work_resumes_in_the_joining_call() {
    for carried in [true, false] {
        let joined = std::panic::AssertUnwindSafe(async move {
            match carried {
                true => spawn_blocking_in_call(|| panic!("blocking boom")).await,
                false => spawn_in_call(async { panic!("task boom") }).await,
            }
        })
        .catch_unwind()
        .await;
        let payload = joined.expect_err("the panic reached the joiner");
        let message = crate::application::tool_panic_scope::payload_message(payload.as_ref());
        assert!(message.ends_with("boom"), "{message}");
    }
}

#[tokio::test]
async fn a_cancelled_task_is_an_error_not_a_panic() {
    let task = spawn_in_call(std::future::pending::<()>());
    task.abort_handle().abort();
    let error = task.await.expect_err("cancelled");
    assert!(error.is_cancelled());
}

#[tokio::test]
async fn blocking_work_on_a_given_runtime_is_joined_the_same_way() {
    let runtime = tokio::runtime::Handle::current();
    let seen = scoped(ToolScope::new("kill"), async move {
        spawn_blocking_in_call_on(&runtime, tool_now).await.unwrap()
    })
    .await;
    assert_eq!(seen.as_deref(), Some("kill"));
}

#[tokio::test]
async fn a_thread_started_in_a_call_runs_in_it_and_its_panic_is_the_calls() {
    let scope = ToolScope::new("find");
    let (seen, panicked) = scoped(scope.clone(), async {
        let seen = std_thread_in_call(std::thread::Builder::new(), tool_now)
            .unwrap()
            .join()
            .unwrap();
        let panicked = std_thread_in_call(std::thread::Builder::new(), || {
            crate::application::tool_panic_scope::current().map(|scope| {
                scope.record_panic(crate::application::tool_panic_scope::PanicSite {
                    message: "in the owner thread".into(),
                    location: None,
                })
            })
        })
        .unwrap()
        .join()
        .unwrap();
        (seen, panicked)
    })
    .await;
    assert_eq!(seen.as_deref(), Some("find"));
    assert!(panicked.is_some(), "the thread saw the call's scope");
    assert_eq!(
        scope.recorded_panic().map(|site| site.message),
        Some("in the owner thread".to_string()),
        "what the hook records there lands on the call"
    );
}
