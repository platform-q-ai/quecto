use super::*;

/// The blocking pool keeps its threads resident: a run of sequential
/// blocking tasks with pauses between them is served by the same one or
/// two threads throughout. A pool that retired its idle thread would start
/// a fresh one — a fresh id — after every pause. Thread identity across
/// two tasks is not the property (#2072): on a loaded runner the pool may
/// start a second worker when the first has not parked yet, and both then
/// stay. A third would take two such stalls overlapping across a pause.
#[test]
fn harness_runtime_keeps_a_resident_blocking_thread() {
    let runtime = build_tokio_runtime().unwrap();
    let run_blocking = || {
        runtime.block_on(async {
            tokio::task::spawn_blocking(|| std::thread::current().id())
                .await
                .unwrap()
        })
    };
    let mut seen = std::collections::HashSet::new();
    for task in 0..9 {
        if task > 0 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        seen.insert(run_blocking());
    }
    assert!(
        seen.len() <= 2,
        "nine sequential tasks were served by {} threads: the pool retires or never reuses",
        seen.len()
    );
}

/// The pauses above prove only that a thread outlives them. This pins the
/// constant the builder sets as its keep-alive to the process lifetime
/// (every blocking thread holds its pid until exit — the accepted price,
/// see `build_tokio_runtime`); that the builder passes it is unguarded:
/// Tokio exposes no stable observable for the pool's keep-alive (#2072).
#[test]
fn blocking_thread_keep_alive_spans_the_process_lifetime() {
    assert!(BLOCKING_THREAD_KEEP_ALIVE >= std::time::Duration::from_secs(60 * 60 * 24 * 365));
}
