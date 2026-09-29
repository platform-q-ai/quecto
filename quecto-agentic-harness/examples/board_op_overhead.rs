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
//! Each arm is the production builder over its own board file, created by
//! the same `bootstrap_run`. After a warm-up, the arms run interleaved,
//! `rounds` times `calls-per-round` calls each, alternating which goes
//! first, and each call is timed alone. It prints, per op, each arm's p50,
//! p95 and mean in microseconds, and the overhead of "on" over "off".
use std::path::Path;
use std::time::Instant;

use quecto::application::swarm::dto::BoardLocation;
use quecto::composition::swarm::{SwarmBoardHandles, board_op_log, build_swarm_board_handles};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::json;

const WARM_UP: usize = 500;
const OPS: [&str; 2] = ["_status", "_snapshot"];

fn handles(dir: &Path, log: Option<&AuditLog>) -> SwarmBoardHandles {
    let location = BoardLocation {
        database: dir.join("swarm.sqlite"),
        checkout: dir.to_path_buf(),
    };
    let handles = build_swarm_board_handles(location, log.and_then(|log| board_op_log(true, log)));
    assert_eq!(handles.telemetry.is_some(), log.is_some());
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
    mean: f64,
}

fn summary(mut samples: Vec<f64>) -> Summary {
    samples.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    Summary {
        p50: quantile(&samples, 0.50),
        p95: quantile(&samples, 0.95),
        mean,
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
    let (off_dir, on_dir) = (
        tempfile::tempdir().expect("a temp dir"),
        tempfile::tempdir().expect("a temp dir"),
    );
    let arms = [
        handles(off_dir.path(), None),
        handles(on_dir.path(), Some(&log)),
    ];
    println!(
        "release: {}, rounds: {rounds}, calls per round: {per_round}",
        !cfg!(debug_assertions)
    );
    println!(
        "| op | off p50 | on p50 | p50 overhead | off p95 | on p95 | p95 overhead | mean overhead |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for op in OPS {
        for arm in &arms {
            for _ in 0..WARM_UP {
                timed(arm, op);
            }
        }
        let mut samples = [Vec::new(), Vec::new()];
        for round in 0..rounds {
            let order = if round % 2 == 0 { [0, 1] } else { [1, 0] };
            for arm in order {
                for _ in 0..per_round {
                    samples[arm].push(timed(&arms[arm], op));
                }
            }
        }
        let [off, on] = samples.map(summary);
        println!(
            "| `{op}` | {:.1} µs | {:.1} µs | {:+.1}% | {:.1} µs | {:.1} µs | {:+.1}% | {:+.1}% |",
            off.p50,
            on.p50,
            percent(on.p50, off.p50),
            off.p95,
            on.p95,
            percent(on.p95, off.p95),
            percent(on.mean, off.mean),
        );
    }
    let written = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:overhead"))
        .expect("the event log was written")
        .lines()
        .count();
    println!("swarm_op lines written with the event log on: {written}");
}
