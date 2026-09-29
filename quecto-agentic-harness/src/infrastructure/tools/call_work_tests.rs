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

/// #2278 review L6: blocking work reaches a thread that may block. From an
/// async worker it runs on the blocking pool, marked; outside any runtime,
/// or on a thread already marked, it runs where it is.
#[tokio::test]
async fn blocking_work_runs_off_the_async_workers() {
    assert!(!may_block(), "a test's async worker may not block");
    let worker = std::thread::current().id();
    let (may, thread) = off_the_workers(|| (may_block(), std::thread::current().id()))
        .await
        .unwrap();
    assert!(may, "the job may block where it runs");
    assert_ne!(thread, worker, "not on the async worker");
    let (may, same) = spawn_blocking_in_call(|| {
        let here = std::thread::current().id();
        futures::executor::block_on(off_the_workers(move || {
            (may_block(), std::thread::current().id() == here)
        }))
        .unwrap()
    })
    .await
    .unwrap();
    assert!(may && same, "already on the blocking pool: run in place");
    let outside =
        std::thread::spawn(|| futures::executor::block_on(off_the_workers(may_block)).unwrap())
            .join()
            .unwrap();
    assert!(outside, "outside any runtime: run in place");
    assert!(off_the_runtime(may_block), "a test's own thread may block");
}

/// Work run here finishes before `block_here` returns, and runs as
/// blocking work, on either runtime flavor and outside any runtime (#2278
/// final review L1, L2).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_run_here_is_blocking_work_on_a_multi_thread_worker() {
    assert!(!may_block(), "an async worker may not block");
    assert!(block_here(may_block));
    assert!(!may_block(), "the mark ends with the work");
}

#[tokio::test(flavor = "current_thread")]
async fn work_run_here_is_blocking_work_on_a_current_thread_runtime() {
    assert!(!may_block(), "an async worker may not block");
    assert!(block_here(may_block));
    assert!(!may_block(), "the mark ends with the work");
}

#[test]
fn work_run_here_outside_any_runtime_runs_here() {
    let caller = std::thread::current().id();
    assert_eq!(block_here(|| std::thread::current().id()), caller);
}
