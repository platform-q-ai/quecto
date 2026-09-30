//! The run watch's board load probe (#2338): what three members' run
//! watches cost the board, idle and beside a writer, with the snapshot poll
//! every watch ran until #2338 (`legacy`: a `_snapshot` every tick) and
//! with the watch of #2338 (`watch`: one `_watch` call a tick, which
//! answers the snapshot only when the board changed or the schedule asked).
//!
//! Opt-in and re-runnable, never part of a test run:
//!
//! ```text
//! cargo run --release -p quecto-agentic-harness \
//!     --example swarm_watch_load -- <legacy|watch> <idle|busy> [seconds]
//! ```
//!
//! Three watchers, each with a board of its own (as each member process
//! has), watch one run in a temporary checkout for `seconds` (default 60).
//! Every watcher calls as the run's coordinator: the operation gate reads
//! only membership, so the load on the file is a member's. `busy` adds a
//! writer creating a task every 100 ms on a board of its own. Every call
//! is recorded in an in-memory event log, and the probe prints, per op,
//! the records, the calls they account for and the busy records, then the
//! watch's `_snapshot` calls per member-minute.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quecto::application::swarm::dto::{BoardLocation, DroppedRecords};
use quecto::application::swarm::ports::{BoardOpLog, CoordinationPort, SessionOpLog};
use quecto::composition::swarm::{SwarmBoard, board_wire, build_swarm_board_handles};
use quecto::domain::swarm::watch::{WATCH_TICK, WatchSchedule};
use quecto::domain::swarm::{BoardOpObservation, ProcessIdentity, SwarmRunSummary};
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use quecto::infrastructure::tools::swarm_bridge::{RunWatch, SwarmContext, process_start};
use serde_json::json;

/// Per op: records, the calls they account for, and busy records.
#[derive(Default)]
struct Counts(Mutex<BTreeMap<String, (u64, u64, u64)>>);

impl BoardOpLog for Counts {
    fn record(&self, observation: BoardOpObservation) {
        let mut counts = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = counts.entry(observation.op.clone()).or_default();
        entry.0 += 1;
        entry.1 += observation.detail.polls.unwrap_or(1);
        entry.2 += u64::from(observation.busy == Some(true));
        // The watch's ticks that answered the snapshot, apart.
        if observation.op == "_watch" && observation.decision.as_deref() == Some("snapshot") {
            let snapshots = counts.entry("_watch/snapshot".into()).or_default();
            snapshots.0 += 1;
            snapshots.1 += 1;
        }
    }

    fn summarize(&self, summary: SwarmRunSummary) {
        std::hint::black_box(summary);
    }
}

impl SessionOpLog for Counts {
    fn dropped(&self, _drops: DroppedRecords) {}

    fn take_unnoted(&self) -> DroppedRecords {
        DroppedRecords::default()
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// A context on a board of its own, recording in `log`.
fn member(checkout: &std::path::Path, log: &Arc<Counts>) -> SwarmContext {
    let board = SwarmBoard::new(build_swarm_board_handles, board_wire());
    assert!(board.record_in(log.clone()));
    SwarmContext {
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        board,
    }
}

/// One watcher until `stop`: `legacy` snapshots every tick; otherwise the
/// watch's one `_watch` call a tick, as its schedule names the cursor.
fn watch(context: &SwarmContext, legacy: bool, stop: &AtomicBool) {
    let mut schedule = WatchSchedule::new();
    while !stop.load(Ordering::Relaxed) {
        match legacy {
            true => {
                let _snapshot = context.snapshot();
            }
            false => {
                let at = now();
                match context.watch(schedule.since(at)) {
                    Ok(RunWatch {
                        event_cursor,
                        snapshot: Some(snapshot),
                    }) => schedule.snapshotted(event_cursor, &snapshot, at),
                    Ok(RunWatch { snapshot: None, .. }) => {}
                    Err(_) => schedule.unreadable(),
                }
            }
        }
        std::thread::sleep(WATCH_TICK);
    }
    context.flush_watch_polls();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let legacy = match args.first().map(String::as_str) {
        Some("legacy") => true,
        Some("watch") => false,
        _ => panic!("usage: swarm_watch_load <legacy|watch> <idle|busy> [seconds]"),
    };
    let busy = match args.get(1).map(String::as_str) {
        Some("busy") => true,
        Some("idle") => false,
        _ => panic!("usage: swarm_watch_load <legacy|watch> <idle|busy> [seconds]"),
    };
    let seconds: u64 = args
        .get(2)
        .map_or(60, |given| given.parse().expect("seconds"));
    let directory = tempfile::tempdir().expect("a temporary checkout");
    std::fs::create_dir_all(directory.path().join(".quecto")).expect("the store's directory");
    let log = Arc::new(Counts::default());
    let creator = member(directory.path(), &log);
    let process = ProcessIdentity {
        pid: std::process::id(),
        started: process_start(std::process::id()).expect("procfs"),
    };
    creator
        .create_run(
            &json!({"goal":"load", "constraints":[], "criteria":[{"id":"t","kind":"command","description":"pass"}], "member_limit":3, "deadline": now() + 3_600.0}),
            &process,
            None,
        )
        .expect("a run");
    log.0.lock().expect("counts").clear();
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let mut threads = Vec::new();
    for _ in 0..3 {
        let (context, stop) = (member(directory.path(), &log), stop.clone());
        threads.push(std::thread::spawn(move || watch(&context, legacy, &stop)));
    }
    if busy {
        let (log, stop) = (log.clone(), stop.clone());
        let location = BoardLocation {
            database: creator.database(),
            checkout: directory.path().to_path_buf(),
        };
        threads.push(std::thread::spawn(move || {
            let handles = build_swarm_board_handles(location, Some(log));
            let mut n = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                let _task = call(
                    &handles,
                    "coordinator",
                    "task_create",
                    json!([format!("r{n}"), "t", ["tests pass"]]),
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }));
    }
    std::thread::sleep(Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        thread.join().expect("a watcher");
    }
    let elapsed = started.elapsed().as_secs_f64();
    let counts = log.0.lock().expect("counts");
    println!(
        "mode={} load={} seconds={elapsed:.1}",
        if legacy { "legacy" } else { "watch" },
        if busy { "busy" } else { "idle" }
    );
    println!("{:<16} {:>8} {:>8} {:>6}", "op", "records", "calls", "busy");
    for (op, (records, calls, busy)) in counts.iter() {
        println!("{op:<16} {records:>8} {calls:>8} {busy:>6}");
    }
    let snapshots = counts.get("_snapshot").map_or(0, |entry| entry.1)
        + counts.get("_watch/snapshot").map_or(0, |entry| entry.1);
    let watch_busy: u64 = ["_snapshot", "_watch"]
        .iter()
        .filter_map(|op| counts.get(*op))
        .map(|entry| entry.2)
        .sum();
    println!(
        "snapshots per member-minute: {:.1}; busy records from the watch: {watch_busy}",
        snapshots as f64 / 3.0 / (elapsed / 60.0)
    );
}
