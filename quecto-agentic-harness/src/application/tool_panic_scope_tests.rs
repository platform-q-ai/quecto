use super::*;

fn scope_tool() -> Option<String> {
    current().map(|scope| scope.tool().to_string())
}

#[test]
fn no_code_runs_in_a_scope_by_default() {
    assert_eq!(scope_tool(), None);
}

#[test]
fn run_in_marks_the_closure_and_restores_after() {
    let scope = ToolScope::new("edit");
    let inside = run_in(&scope, scope_tool);
    assert_eq!(inside.as_deref(), Some("edit"));
    assert_eq!(scope_tool(), None);
}

#[test]
fn a_nested_scope_restores_the_outer_one() {
    let outer = ToolScope::new("workflow");
    let inner = ToolScope::new("edit");
    let seen = run_in(&outer, || {
        let nested = run_in(&inner, scope_tool);
        (nested, scope_tool())
    });
    assert_eq!(
        seen,
        (Some("edit".to_string()), Some("workflow".to_string()))
    );
}

#[test]
fn an_unwinding_panic_restores_the_previous_scope() {
    let scope = ToolScope::new("edit");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_in(&scope, || panic!("inside"));
    }));
    assert!(outcome.is_err());
    assert_eq!(scope_tool(), None, "the scope ended with the unwind");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scoped_future_is_in_scope_on_every_poll_across_awaits_and_threads() {
    let scope = ToolScope::new("grep");
    let future = scoped(scope, async {
        let mut seen = vec![scope_tool()];
        for _ in 0..20 {
            tokio::task::yield_now().await;
            seen.push(scope_tool());
        }
        seen
    });
    // Spawned on a multi-thread runtime, the task may resume on either worker.
    let seen = tokio::spawn(future).await.unwrap();
    assert!(
        seen.iter().all(|tool| tool.as_deref() == Some("grep")),
        "{seen:?}"
    );
    assert_eq!(scope_tool(), None, "the caller is not in the scope");
}

#[tokio::test]
async fn a_scoped_future_does_not_leak_its_scope_between_polls() {
    let scope = ToolScope::new("grep");
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let scoped_task = tokio::spawn(scoped(scope, async move {
        let _ = rx.await;
        scope_tool()
    }));
    tokio::task::yield_now().await;
    // The scoped task is parked; this task, on the same thread, is unmarked.
    assert_eq!(scope_tool(), None);
    tx.send(()).unwrap();
    assert_eq!(scoped_task.await.unwrap().as_deref(), Some("grep"));
}

#[tokio::test]
async fn carried_blocking_work_runs_in_the_callers_scope() {
    let scope = ToolScope::new("grep");
    let seen = scoped(scope, async {
        let carried = tokio::task::spawn_blocking(carry(scope_tool))
            .await
            .unwrap();
        let bare = tokio::task::spawn_blocking(scope_tool).await.unwrap();
        (carried, bare)
    })
    .await;
    assert_eq!(seen, (Some("grep".to_string()), None));
}

#[tokio::test]
async fn a_carried_future_runs_in_the_callers_scope_and_a_bare_one_does_not() {
    let scope = ToolScope::new("bash");
    let seen = scoped(scope, async {
        let carried = tokio::spawn(carry_future(async { scope_tool() }));
        let bare = tokio::spawn(async { scope_tool() });
        (carried.await.unwrap(), bare.await.unwrap())
    })
    .await;
    assert_eq!(seen, (Some("bash".to_string()), None));
}

#[test]
fn carry_outside_a_scope_runs_the_job_unmarked() {
    assert_eq!(carry(scope_tool)(), None);
}

#[test]
fn the_first_recorded_panic_is_kept() {
    let scope = ToolScope::new("edit");
    let site = |message: &str| PanicSite {
        message: message.into(),
        location: Some("src/x.rs:1:2".into()),
    };
    scope.record_panic(site("first"));
    scope.record_panic(site("second"));
    assert_eq!(scope.recorded_panic(), Some(site("first")));
}

#[test]
#[should_panic(expected = "a tool scope names its tool")]
fn a_scope_needs_a_tool_name() {
    let _ = ToolScope::new("");
}

#[tokio::test]
async fn a_scoped_call_is_in_flight_until_it_is_dropped_and_carried_work_is_not_counted() {
    let tool = "in_flight_probe_tool";
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let call = tokio::spawn(scoped(ToolScope::new(tool), async move {
        let carried = carry_future(async {});
        let during = in_flight_tools();
        drop(carried);
        let _ = rx.await;
        during
    }));
    tokio::task::yield_now().await;
    let listed = |tools: &[String]| tools.iter().filter(|name| *name == tool).count();
    assert_eq!(listed(&in_flight_tools()), 1, "one entry while it runs");
    tx.send(()).unwrap();
    let during = call.await.unwrap();
    assert_eq!(listed(&during), 1, "carried work adds no entry");
    assert_eq!(listed(&in_flight_tools()), 0, "gone once the call ends");
}

#[tokio::test]
async fn a_calls_scope_closes_when_the_call_ends_so_its_leftover_work_is_outside_it() {
    let scope = ToolScope::new("bash");
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    scoped(scope.clone(), async move {
        // A reader the call leaves running when it returns.
        tokio::spawn(carry_future(async move {
            let during = scope_tool();
            let _ = rx.await;
            let _ = seen_tx.send((during, scope_tool()));
        }));
        tokio::task::yield_now().await;
    })
    .await;
    assert!(!scope.is_open(), "the call ended");
    tx.send(()).unwrap();
    let (during, after) = seen_rx.await.unwrap();
    assert_eq!(during.as_deref(), Some("bash"));
    assert_eq!(after, None, "a closed scope contains nothing");
}

#[test]
#[should_panic(expected = "a call runs in a fresh scope")]
fn a_closed_scope_cannot_run_another_call() {
    let scope = ToolScope::new("bash");
    drop(scoped(scope.clone(), async {}));
    drop(scoped(scope, async {}));
}

#[test]
fn a_second_panic_before_the_first_is_caught_is_told_apart() {
    let scope = ToolScope::new("edit");
    run_in(&scope, || {
        assert!(!begin_contained_unwind(), "the first contained unwind");
        assert!(begin_contained_unwind(), "a second one before it ended");
    });
    // Left with no panic in progress: that unwind is over.
    assert!(!begin_contained_unwind());
    end_contained_unwind();
    run_in(&scope, || {});
    assert!(
        !begin_contained_unwind(),
        "any scope exit with no panic ends it"
    );
    end_contained_unwind();
}

/// A-5: a scope a destructor enters and leaves while a contained panic
/// unwinds through outer frames does not end that unwind, so a second panic
/// after it is still told apart.
#[test]
fn a_nested_scope_left_during_the_unwind_does_not_end_it() {
    struct SeesTheUnwind(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for SeesTheUnwind {
        fn drop(&mut self) {
            // A nested scope entered and left by a destructor mid-unwind.
            run_in(&ToolScope::new("inner"), || {});
            self.0.store(
                begin_contained_unwind(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }
    }
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let outer = ToolScope::new("outer");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_in(&outer, || {
            let _watch = SeesTheUnwind(seen.clone());
            run_in(&ToolScope::new("inner"), || {
                assert!(!begin_contained_unwind(), "the hook lets it unwind");
                panic!("the call's own panic");
            });
        });
    }));
    assert!(caught.is_err());
    assert!(
        seen.load(std::sync::atomic::Ordering::SeqCst),
        "still unwinding when the destructor ran"
    );
    end_contained_unwind();
}

#[test]
fn the_recorded_panic_can_be_read_without_taking_it() {
    let scope = ToolScope::new("edit");
    assert_eq!(scope.recorded_panic(), None);
    let site = PanicSite {
        message: "m".into(),
        location: None,
    };
    scope.record_panic(site.clone());
    assert_eq!(scope.recorded_panic(), Some(site.clone()));
    assert_eq!(scope.recorded_panic(), Some(site), "reading leaves it");
}

/// A-5: a panic caught outside the scope leaves no stale flag for the next
/// scope entered on the thread: its first panic is containable again.
#[test]
fn a_scope_entered_after_a_caught_panic_starts_fresh() {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_in(&ToolScope::new("edit"), || {
            assert!(!begin_contained_unwind(), "the hook lets it unwind");
            panic!("caught outside the scope");
        });
    }));
    assert!(caught.is_err());
    run_in(&ToolScope::new("next"), || {
        assert!(!begin_contained_unwind(), "a fresh first panic");
    });
    end_contained_unwind();
}

/// What the hook does for a contained panic, without installing it: record
/// the site on the scope and mark the unwind as contained.
fn as_the_hook_would(scope: &ToolScope, message: &str) {
    scope.record_panic(PanicSite {
        message: message.to_string(),
        location: None,
    });
    assert!(!begin_contained_unwind(), "the first panic on this thread");
}

#[test]
fn a_panic_caught_in_the_call_is_over_and_forgotten() {
    let scope = ToolScope::new("edit");
    run_in(&scope, || {
        let caught = catch_in_call(|| -> u8 {
            as_the_hook_would(&scope, "handled inside the tool");
            panic!("handled inside the tool")
        });
        assert!(caught.is_err(), "the panic was caught");
        assert_eq!(
            scope.recorded_panic(),
            None,
            "a handled panic is not the call's"
        );
        // A later, real panic is the first again: contained, not fatal.
        assert!(!begin_contained_unwind(), "the caught unwind is over");
        end_contained_unwind();
    });
}

#[test]
fn a_call_that_catches_nothing_leaves_the_record_as_it_was() {
    let scope = ToolScope::new("edit");
    run_in(&scope, || {
        assert_eq!(catch_in_call(|| 7).ok(), Some(7));
        assert_eq!(scope.recorded_panic(), None);
        scope.record_panic(PanicSite {
            message: "earlier".into(),
            location: None,
        });
        assert_eq!(catch_in_call(|| 8).ok(), Some(8));
        assert_eq!(scope.recorded_panic().unwrap().message, "earlier");
    });
}

#[test]
fn a_catch_forgets_only_its_own_threads_panic_not_carried_works() {
    let scope = ToolScope::new("find");
    run_in(&scope, || {
        let caught = catch_in_call(|| -> u8 {
            // Carried work on another thread panics while this one runs.
            let carried = scope.clone();
            std::thread::spawn(move || run_in(&carried, || as_the_hook_would(&carried, "carried")))
                .join()
                .unwrap();
            as_the_hook_would(&scope, "mine");
            panic!("mine")
        });
        assert!(caught.is_err());
    });
    assert_eq!(
        scope.recorded_panic().unwrap().message,
        "carried",
        "another thread's panic stays the call's"
    );
}

#[test]
fn a_catch_keeps_a_panic_recorded_before_it_ran() {
    let scope = ToolScope::new("edit");
    run_in(&scope, || {
        scope.record_panic(PanicSite {
            message: "before".into(),
            location: None,
        });
        let caught = catch_in_call(|| -> u8 { panic!("later, handled") });
        assert!(caught.is_err());
        assert_eq!(scope.recorded_panic().unwrap().message, "before");
    });
}

#[test]
fn a_catch_outside_any_scope_only_catches() {
    let caught = catch_in_call(|| -> u8 { panic!("no scope") });
    assert!(caught.is_err());
    assert_eq!(scope_tool(), None);
}

/// #2192 review L4: the recording thread's catch must not lose a panic
/// another thread (a detached `find` owner) raised in the same call after it.
#[test]
fn a_catch_keeps_a_panic_another_thread_raised_after_its_own() {
    let scope = ToolScope::new("find");
    run_in(&scope, || {
        let caught = catch_in_call(|| -> u8 {
            as_the_hook_would(&scope, "mine");
            let carried = scope.clone();
            std::thread::spawn(move || run_in(&carried, || as_the_hook_would(&carried, "carried")))
                .join()
                .unwrap();
            panic!("mine")
        });
        assert!(caught.is_err());
    });
    assert_eq!(
        scope.recorded_panic().map(|site| site.message).as_deref(),
        Some("carried"),
        "the other thread's panic still fails the call"
    );
}

#[test]
fn a_catch_forgets_its_own_panic_even_when_another_thread_recorded_first() {
    let scope = ToolScope::new("find");
    let carried = scope.clone();
    std::thread::spawn(move || run_in(&carried, || as_the_hook_would(&carried, "carried")))
        .join()
        .unwrap();
    run_in(&scope, || {
        let caught = catch_in_call(|| -> u8 {
            as_the_hook_would(&scope, "mine");
            panic!("mine")
        });
        assert!(caught.is_err());
        assert!(!scope.has_own_panic(), "the handled panic is forgotten");
    });
    assert_eq!(scope.recorded_panic().unwrap().message, "carried");
}

fn recorded(message: &str, thread: u64) -> Recorded {
    Recorded {
        site: PanicSite {
            message: message.into(),
            location: None,
        },
        thread,
    }
}

#[test]
fn a_scope_remembers_one_panic_per_thread_and_pins_on_overflow() {
    let mut records = Records::default();
    assert!(records.record(recorded("first", 1)));
    assert!(!records.record(recorded("consequence", 1)), "same thread");
    let threads = 2..(2 + MAX_LATER_THREADS as u64 + 1);
    for thread in threads.clone() {
        assert!(!records.record(recorded(&format!("t{thread}"), thread)));
    }
    assert_eq!(records.later.len(), MAX_LATER_THREADS, "bounded");
    // Every live thread forgets its own; the overflow pinned one stays.
    for thread in std::iter::once(1).chain(threads) {
        drop(records.forget(thread));
    }
    let left = records.kept.as_ref().expect("the scope stays failed");
    assert_eq!(left.thread, PINNED_THREAD);
    assert_eq!(left.site.message, format!("t{}", 1 + MAX_LATER_THREADS));
    assert!(
        records.forget(PINNED_THREAD).is_none(),
        "nothing forgets it"
    );
    assert!(records.kept.is_some());
}

#[test]
fn forgetting_the_kept_panic_promotes_the_earliest_other_thread() {
    let mut records = Records::default();
    records.record(recorded("a", 1));
    records.record(recorded("b", 2));
    records.record(recorded("c", 3));
    assert_eq!(records.forget(2).map(|r| r.site.message), None, "not kept");
    assert_eq!(
        records.forget(1).map(|r| r.site.message).as_deref(),
        Some("a")
    );
    assert_eq!(records.kept.as_ref().unwrap().site.message, "c");
    assert!(records.later.is_empty());
    assert!(records.forget(3).is_some());
    assert!(records.kept.is_none());
}

#[test]
fn a_torn_down_thread_forgets_nothing() {
    let mut records = Records::default();
    records.record(recorded("torn down", PINNED_THREAD));
    assert!(records.forget(PINNED_THREAD).is_none());
    assert!(records.kept.is_some());
}

/// #2192 review (PRRT_kwDORUxnPM6mf9Ba): once a call's scope is closed no
/// panic is recorded on it — the hook then treats the panic as fatal — so
/// the call's backstop, which reads after the close, can never miss one.
#[test]
fn a_closed_scope_takes_no_record() {
    let scope = ToolScope::new("find");
    futures::executor::block_on(scoped(scope.clone(), async {}));
    assert!(!scope.is_open());
    let outcome = run_in(&scope, || {
        scope.record_panic(PanicSite {
            message: "after the call ended".into(),
            location: None,
        })
    });
    assert_eq!(outcome, Recording::Refused);
    assert_eq!(scope.recorded_panic(), None);
}

/// Closing waits for a record already in progress, so a record either lands
/// before the close returns (and the backstop sees it) or is refused.
#[test]
fn closing_waits_for_a_record_in_progress() {
    let scope = ToolScope::new("find");
    let recording = scope.site.lock().unwrap();
    let (closed, done) = std::sync::mpsc::channel();
    let closer = scope.clone();
    let thread = std::thread::spawn(move || {
        closer.close();
        closed.send(()).unwrap();
    });
    assert!(
        done.recv_timeout(std::time::Duration::from_millis(200))
            .is_err(),
        "the close waits for the record"
    );
    drop(recording);
    done.recv_timeout(std::time::Duration::from_secs(10))
        .expect("the close finishes once the record is done");
    thread.join().unwrap();
    assert!(!scope.is_open());
}

#[test]
fn a_record_says_whether_it_was_kept() {
    let scope = ToolScope::new("edit");
    let site = |message: &str| PanicSite {
        message: message.into(),
        location: None,
    };
    assert_eq!(scope.record_panic(site("first")), Recording::Kept);
    assert_eq!(scope.record_panic(site("second")), Recording::Remembered);
}
