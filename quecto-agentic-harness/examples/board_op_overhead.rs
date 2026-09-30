//! The board op telemetry overhead probe (#2303): what recording a
//! `swarm_op` per call costs, with the event log off (the default, owner
//! decision T1) and on, writing to a real `AuditLog`.
//!
//! Opt-in and re-runnable, never part of a test run:
//!
//! ```text
//! cargo run --release -p quecto-agentic-harness --features test-support \
//!     --example board_op_overhead [rounds] [calls-per-round]
//! ```
//!
//! Three arms, each the production graph over its own board file, created
//! by the same `bootstrap_run`: `off` (no event log), `meter` (measured,
//! the record discarded: what measuring alone costs) and `on` (measured
//! and written to a real `AuditLog`). After a warm-up, the arms run
//! interleaved, `rounds` times `calls-per-round` calls each, rotating which
//! goes first, and each call is timed alone. It prints, per op, each arm's
//! p50 and p95 in microseconds and their overhead over `off`.
use std::path::Path;
use std::time::Instant;

use std::sync::Arc;

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::BoardOpLog;
use quecto::composition::swarm::{
    SwarmBoardHandles, board_op_log, build_swarm_board_handles, with_event_log,
};
use quecto::domain::swarm::BoardOpObservation;
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::swarm_board::ids::Uuid4Ids;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use quecto::infrastructure::tools::swarm_lifecycle::SystemClock;
use quecto::infrastructure::workspace::checkout_paths::ResolvedCheckout;
use serde_json::json;

const WARM_UP: usize = 500;
const OPS: [&str; 2] = ["_status", "_snapshot"];

/// A log that discards every record: the `meter` arm's.
struct Discarded;

impl BoardOpLog for Discarded {
    fn record(&self, observation: BoardOpObservation) {
        std::hint::black_box(observation);
    }

    fn summarize(&self, summary: quecto::domain::swarm::SwarmRunSummary) {
        std::hint::black_box(summary);
    }
}

/// How an arm records.
enum Arm<'a> {
    Off,
    Meter,
    On(&'a AuditLog),
}

fn handles(dir: &Path, arm: &Arm<'_>) -> SwarmBoardHandles {
    let location = BoardLocation {
        database: dir.join("swarm.sqlite"),
        checkout: dir.to_path_buf(),
    };
    let handles = match arm {
        Arm::Off => build_swarm_board_handles(location, None),
        Arm::Meter => with_event_log(
            SqliteBoardRepository::new(&location),
            Arc::new(SystemClock),
            Arc::new(Uuid4Ids),
            Arc::new(ResolvedCheckout::new(dir)),
            Arc::new(Discarded),
        ),
        Arm::On(log) => build_swarm_board_handles(
            location,
            board_op_log(true, log)
                .map(|log| log as Arc<dyn quecto::application::swarm::ports::BoardOpLog>),
        ),
    };
    assert_eq!(handles.telemetry.is_some(), !matches!(arm, Arm::Off));
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).expect("bootstrap");
    handles
}

/// One call of `op`, in microseconds.
fn timed(handles: &SwarmBoardHandles, op: &str) -> f64 {
    let started = Instant::now();
    let answer = call(handles, "parent", op, json!([]));
    let elapsed = started.elapsed();
    assert!(answer.is_ok(), "{op}: {answer:?}");
    elapsed.as_secs_f64() * 1e6
}

/// The `q` quantile of sorted `samples` (nearest rank).
fn quantile(samples: &[f64], q: f64) -> f64 {
    let rank = (q * samples.len() as f64).ceil() as usize;
    samples[rank.clamp(1, samples.len()) - 1]
}

struct Summary {
    p50: f64,
    p95: f64,
}

fn summary(mut samples: Vec<f64>) -> Summary {
    samples.sort_by(f64::total_cmp);
    Summary {
        p50: quantile(&samples, 0.50),
        p95: quantile(&samples, 0.95),
    }
}

fn percent(on: f64, off: f64) -> f64 {
    (on / off - 1.0) * 100.0
}

fn argument(position: usize, default: usize) -> usize {
    std::env::args()
        .nth(position)
        .map_or(default, |value| value.parse().expect("a count"))
}

fn main() {
    let (rounds, per_round) = (argument(1, 10), argument(2, 300));
    let base = tempfile::tempdir().expect("a temp dir");
    let log = AuditLog::open_sync(base.path(), "cli:overhead").expect("the event log");
    let dirs: Vec<_> = (0..3)
        .map(|_| tempfile::tempdir().expect("a temp dir"))
        .collect();
    let arms = [
        handles(dirs[0].path(), &Arm::Off),
        handles(dirs[1].path(), &Arm::Meter),
        handles(dirs[2].path(), &Arm::On(&log)),
    ];
    println!(
        "release: {}, rounds: {rounds}, calls per round: {per_round}",
        !cfg!(debug_assertions)
    );
    println!("| op | quantile | off | meter | on (real log) | meter overhead | on overhead |");
    println!("|---|---|---|---|---|---|---|");
    for op in OPS {
        for arm in &arms {
            for _ in 0..WARM_UP {
                timed(arm, op);
            }
        }
        let mut samples = [Vec::new(), Vec::new(), Vec::new()];
        for round in 0..rounds {
            for turn in 0..arms.len() {
                let arm = (round + turn) % arms.len();
                for _ in 0..per_round {
                    samples[arm].push(timed(&arms[arm], op));
                }
            }
        }
        let [off, meter, on] = samples.map(summary);
        for (name, pick) in [("p50", 0), ("p95", 1)] {
            let value = |summary: &Summary| [summary.p50, summary.p95][pick];
            println!(
                "| `{op}` | {name} | {:.1} µs | {:.1} µs | {:.1} µs | {:+.1}% | {:+.1}% |",
                value(&off),
                value(&meter),
                value(&on),
                percent(value(&meter), value(&off)),
                percent(value(&on), value(&off)),
            );
        }
    }
    let written = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:overhead"))
        .expect("the event log was written")
        .lines()
        .count();
    println!("swarm_op lines written with the event log on: {written}");
}
