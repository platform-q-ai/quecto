//! The `_record_request` latency probe (#2340): where one recorded model
//! request's board call spends its time, and whether that grows with the
//! run's length.
//!
//! Opt-in and re-runnable, never part of a test run:
//!
//! ```text
//! cargo run --release -p quecto-agentic-harness --features test-support \
//!     --example record_request_latency [board-parent-dir] [requests]
//! ```
//!
//! The board is created under `board-parent-dir` (default: the system temp
//! dir), so running it once on a disk-backed directory and once on tmpfs
//! separates the commit's `fsync` from the rest. It prints, per window of
//! the run, the p50 and p95 of `_record_request` and of `_status` (a read
//! of the same board), and the raw SQLite cost of a committed one-row
//! insert under the board's pragmas (`synchronous` left at its default)
//! against the same insert with `synchronous=OFF`, on the same directory.
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use quecto::application::swarm::dto::BoardLocation;
use quecto::composition::swarm::{SwarmBoardHandles, build_swarm_board_handles};
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use serde_json::{Value, json};

const WINDOW: usize = 100;

/// A record shaped like the harness's (`RequestObservation` plus
/// `runtime`), about 1 KiB encoded.
fn observation(index: usize) -> Value {
    json!({
        "request_id": format!("{index:08}-7c1d-4f7e-9d2a-5b6c7d8e9f00"),
        "started_unix_ms": 1_790_000_000_000_u64 + index as u64,
        "finished_unix_ms": 1_790_000_004_000_u64 + index as u64,
        "attempt_diagnostics": [{"attempt": 1, "status": 200, "duration_ms": 4000,
            "first_byte_ms": 900, "retry_after_ms": null, "transport": "https"}],
        "model": "claude-opus-5-5", "provider": "anthropic", "outcome": "succeeded",
        "error_class": null, "input_tokens": 1200, "context_input_tokens": 41_000,
        "output_tokens": 800, "cache_read_tokens": 39_000, "cache_write_tokens": 700,
        "estimated_cost_micro_usd": 21_000, "estimated_context_tokens": 41_500,
        "instrumented_attempts": 1, "oauth_retries": 0, "duration_ms": 4000,
        "first_token_ms": 900,
        "harness_prefix_sha256": "0f".repeat(32), "harness_prefix_bytes": 18_000,
        "harness_prefix_unchanged": true,
        "runtime": {"process_instance_id": "7f0c1e7e-8b6a-4c3e-9a55-1f2e3d4c5b6a",
            "package_version": "0.107.190", "build_source_revision": "a".repeat(40),
            "build_dirty": false, "executable_digest_pending": false,
            "executable_sha256": "ab".repeat(32)},
    })
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs_f64()
}

fn handles(dir: &Path) -> SwarmBoardHandles {
    let location = BoardLocation {
        database: dir.join("swarm.sqlite"),
        checkout: dir.to_path_buf(),
    };
    let handles = build_swarm_board_handles(location, None);
    let run = json!({
        "goal": "ship", "constraints": [],
        "criteria": [{"id": "t", "kind": "command", "description": "test"}],
        "member_limit": 3, "deadline": now() + 86_400.0,
    });
    call(&handles, "parent", "create_run", run).expect("create_run");
    handles
}

/// One call, in microseconds.
fn timed(handles: &SwarmBoardHandles, method: &str, args: Value) -> f64 {
    let started = Instant::now();
    let answer = call(handles, "parent", method, args);
    let elapsed = started.elapsed();
    assert!(answer.is_ok(), "{method}: {answer:?}");
    elapsed.as_secs_f64() * 1e6
}

/// The `q` quantile of `samples` (nearest rank).
fn quantile(samples: &[f64], q: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The committed one-row insert's cost on a board-like file in `dir`,
/// with `synchronous` as given (`None`: SQLite's default, as the board).
fn raw_insert(dir: &Path, synchronous: Option<&str>, rounds: usize) -> Vec<f64> {
    let path = dir.join(format!("raw-{}.sqlite", synchronous.unwrap_or("default")));
    let mut connection = rusqlite::Connection::open(&path).expect("open");
    connection
        .execute_batch("CREATE TABLE t (id TEXT PRIMARY KEY, payload TEXT)")
        .expect("schema");
    if let Some(mode) = synchronous {
        connection
            .execute_batch(&format!("PRAGMA synchronous={mode}"))
            .expect("pragma");
    }
    let payload = observation(0).to_string();
    (0..rounds)
        .map(|index| {
            let started = Instant::now();
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .expect("begin");
            transaction
                .execute(
                    "INSERT INTO t VALUES(?,?)",
                    rusqlite::params![index.to_string(), payload],
                )
                .expect("insert");
            transaction.commit().expect("commit");
            started.elapsed().as_secs_f64() * 1e6
        })
        .collect()
}

/// Each read `_record_request` runs over the ledger, timed alone on the
/// board the run left (no transaction, no commit).
fn breakdown(board: &Path) {
    const AGGREGATES: &str = "count(*) requests, coalesce(sum(tokens),0) observed_tokens, coalesce(sum(unknown),0) unknown_usage_requests, coalesce(sum(attempts),0) attempts, coalesce(sum(input_tokens),0) reported_input_tokens, coalesce(sum(output_tokens),0) reported_output_tokens, coalesce(sum(cache_read_tokens),0) reported_cache_read_tokens, coalesce(sum(cache_write_tokens),0) reported_cache_write_tokens, count(cache_read_tokens) cache_read_known_requests, count(cache_write_tokens) cache_write_known_requests";
    let connection = rusqlite::Connection::open(board).expect("open the board");
    let statements = [
        ("totals", format!("SELECT {AGGREGATES} FROM request_usage")),
        (
            "per member",
            format!(
                "SELECT actor member, {AGGREGATES} FROM request_usage GROUP BY actor ORDER BY actor"
            ),
        ),
        (
            "recent ten",
            "SELECT actor,payload FROM request_usage ORDER BY rowid DESC LIMIT 10".to_owned(),
        ),
        ("row count", "SELECT count(*) FROM request_usage".to_owned()),
        (
            "two totals",
            "SELECT coalesce(sum(tokens),0), coalesce(sum(unknown),0) FROM request_usage"
                .to_owned(),
        ),
    ];
    for (name, sql) in statements {
        let mut statement = connection.prepare(&sql).expect("prepare");
        let samples: Vec<f64> = (0..200)
            .map(|_| {
                let started = Instant::now();
                let rows = statement
                    .query_map([], |row| row.get::<_, rusqlite::types::Value>(0))
                    .expect("query")
                    .count();
                std::hint::black_box(rows);
                started.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        println!(
            "statement {name}: p50 {:.0} µs, p95 {:.0} µs",
            quantile(&samples, 0.5),
            quantile(&samples, 0.95)
        );
    }
}

fn main() {
    let parent = std::env::args()
        .nth(1)
        .map_or_else(std::env::temp_dir, Into::into);
    let requests: usize = std::env::args()
        .nth(2)
        .map_or(1_000, |value| value.parse().expect("a count"));
    let dir = tempfile::tempdir_in(&parent).expect("a temp dir");
    let handles = handles(dir.path());
    println!(
        "release: {}, board under {}, requests: {requests}",
        !cfg!(debug_assertions),
        parent.display()
    );
    println!("| requests recorded | `_record_request` p50 | p95 | `_status` p50 | p95 |");
    println!("|---|---|---|---|---|");
    let (mut records, mut reads) = (Vec::new(), Vec::new());
    for index in 0..requests {
        records.push(timed(
            &handles,
            "_record_request",
            json!([observation(index)]),
        ));
        reads.push(timed(&handles, "_status", json!([])));
        if records.len() == WINDOW {
            println!(
                "| {} | {:.0} µs | {:.0} µs | {:.0} µs | {:.0} µs |",
                index + 1,
                quantile(&records, 0.5),
                quantile(&records, 0.95),
                quantile(&reads, 0.5),
                quantile(&reads, 0.95),
            );
            records.clear();
            reads.clear();
        }
    }
    breakdown(&dir.path().join("swarm.sqlite"));
    for synchronous in [None, Some("OFF")] {
        let samples = raw_insert(dir.path(), synchronous, 200);
        println!(
            "raw committed insert, synchronous={}: p50 {:.0} µs, p95 {:.0} µs",
            synchronous.unwrap_or("default (FULL)"),
            quantile(&samples, 0.5),
            quantile(&samples, 0.95),
        );
    }
}
