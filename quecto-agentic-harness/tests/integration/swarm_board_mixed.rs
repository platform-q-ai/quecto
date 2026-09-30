//! Mixed writers on one board file (#2274; S13 extends it): two board
//! handles, each with its own connections, id counter and clock, take turns
//! on the **same** SQLite file, as the members of a swarm do. Until #2283
//! one of them was the Python board; with it deleted, the other writer is a
//! second Rust board, and the Python side stands in the bytes it wrote
//! (`PENDING`, `KNOWN`, `budget_text`: Python's `encode` and `json.dumps`)
//! and in the golden fixtures the single-writer sequences are compared
//! with. A redelivery is compared by decoded value (Python's `==`), so these
//! tests also read the raw rows: each writer must store the very bytes
//! Python's `encode` (or, for the budget, `json.dumps`) wrote, and neither
//! may write where the other would not.
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_golden, step_text};

/// One board file two writers open.
struct Mixed {
    _dir: tempfile::TempDir,
    database: PathBuf,
    other: RustBoard,
    rust: RustBoard,
}

impl Mixed {
    /// A running run coordinated by `parent` (created by the other
    /// writer), with `worker` live.
    fn running() -> Self {
        let dir = tempfile::tempdir().expect("a directory for the board");
        let checkout = dir.path().join("board");
        std::fs::create_dir_all(&checkout).expect("create the board directory");
        let database = checkout.join("swarm.sqlite");
        let other = RustBoard::open(&database, &checkout);
        let rust = RustBoard::open(&database, &checkout);
        let create = json!({
            "goal": "mixed writers",
            "constraints": [],
            "criteria": [{"id": "t", "kind": "command", "description": "test"}],
            "member_limit": 5,
            "deadline": NOW + 3_600.0,
        });
        for (method, args, offset) in [
            ("create_run", create, 0.0),
            ("_admit", json!(["worker", "res-w"]), 1.0),
            ("_activate", json!(["worker", "res-w", 11, "t", null]), 2.0),
        ] {
            let outcome = other.call("parent", method, &args, NOW + offset);
            assert!(matches!(outcome, Outcome::Ok(_)), "{method}: {outcome:?}");
        }
        assert_board(&database);
        Self {
            _dir: dir,
            database,
            other,
            rust,
        }
    }

    fn other_records(&self, record: &Value, offset: f64) -> Outcome {
        self.other
            .call("worker", "_record_request", &json!([record]), NOW + offset)
    }

    fn rust_records(&self, record: &Value, offset: f64) -> Outcome {
        self.rust
            .call("worker", "_record_request", &json!([record]), NOW + offset)
    }

    /// The one text cell `sql` selects, read from the file as stored.
    fn stored_text(&self, sql: &str) -> String {
        let connection =
            Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open the shared board read-only");
        connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
    }

    /// The shared request's stored payload.
    fn request_payload(&self) -> String {
        self.stored_text("SELECT payload FROM request_usage WHERE request_id='shared'")
    }

    /// The stored budget payload.
    fn budget_payload(&self) -> String {
        self.stored_text("SELECT payload FROM usage_budget WHERE id=1")
    }

    /// How many events named `action` the file holds.
    fn events(&self, action: &str) -> String {
        self.stored_text(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM events WHERE action='{action}'"
        ))
    }

    /// The report both writers read of the one file: identical.
    fn report(&self, offset: f64) -> Value {
        let other = self
            .other
            .call("parent", "usage_report", &json!([]), NOW + offset);
        let rust = self
            .rust
            .call("parent", "usage_report", &json!([]), NOW + offset);
        assert_eq!(other, rust, "both read the same report");
        match rust {
            Outcome::Ok(report) => report,
            other => panic!("usage_report: {other:?}"),
        }
    }
}

fn assert_board(database: &Path) {
    assert!(database.exists(), "the shared board file exists");
}

/// A record whose runtime's digest is still pending, and the same record
/// once the digest is known.
fn records() -> (Value, Value) {
    let pending = json!({"request_id": "shared", "model": "m\u{e9}", "duration_ms": 12,
        "context_input_tokens": 30, "output_tokens": 7, "input_tokens": 25,
        "instrumented_attempts": 1, "outcome": "succeeded", "extra": [1.5, 1e16, 0.1, {"b": 1, "a": null}],
        "runtime": {"process_instance_id": "p", "executable_digest_pending": true}});
    let mut known = pending.clone();
    known["runtime"] = json!({"process_instance_id": "p", "executable_digest_pending": false,
        "executable_sha256": "abc"});
    (pending, known)
}

/// `encode(pending)`, as Python's `json.dumps(sort_keys=True,
/// separators=(',', ':'))` writes it: sorted keys at every depth, `é`
/// escaped, `1e16` as `1e+16`.
const PENDING: &str = r#"{"context_input_tokens":30,"duration_ms":12,"extra":[1.5,1e+16,0.1,{"a":null,"b":1}],"input_tokens":25,"instrumented_attempts":1,"model":"m\u00e9","outcome":"succeeded","output_tokens":7,"request_id":"shared","runtime":{"executable_digest_pending":true,"process_instance_id":"p"}}"#;
/// `encode(known)`.
const KNOWN: &str = r#"{"context_input_tokens":30,"duration_ms":12,"extra":[1.5,1e+16,0.1,{"a":null,"b":1}],"input_tokens":25,"instrumented_attempts":1,"model":"m\u00e9","outcome":"succeeded","output_tokens":7,"request_id":"shared","runtime":{"executable_digest_pending":false,"executable_sha256":"abc","process_instance_id":"p"}}"#;

/// A request the other writer recorded (as Python's `encode` wrote it)
/// is redelivered to this one.
#[test]
fn a_request_another_writer_recorded_is_redelivered_here() {
    let board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(board.other_records(&pending, 3.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "Python's encode, byte for byte"
    );
    assert!(matches!(board.rust_records(&pending, 4.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "no digest: nothing written"
    );
    let report = board.report(5.0);
    assert_eq!(report["totals"]["requests"], json!(1));
    assert!(matches!(board.rust_records(&known, 6.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Rust's encode, Python's bytes"
    );
    let report = board.report(7.0);
    assert_eq!(
        report["recent_requests"][0]["observation"]["runtime"]["executable_sha256"],
        json!("abc")
    );
    assert!(matches!(board.other_records(&known, 8.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "the other writer rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.rust_records(&changed, 9.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.request_payload(), KNOWN);
}

/// And the reverse: this writer records, the other redelivers.
#[test]
fn a_request_recorded_here_is_redelivered_by_another_writer() {
    let board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(board.rust_records(&pending, 3.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "Rust's encode, Python's bytes"
    );
    assert!(matches!(board.other_records(&pending, 4.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "no digest: nothing written"
    );
    assert_eq!(board.report(5.0)["totals"]["requests"], json!(1));
    assert!(matches!(board.other_records(&known, 6.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Python's encode, byte for byte"
    );
    assert!(matches!(board.rust_records(&known, 7.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Rust rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.other_records(&changed, 8.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.report(9.0)["totals"]["observed_tokens"], json!(37));
}

/// A request of `tokens` observed tokens (context input and output), as
/// the worker's `id`.
fn measured(id: &str, tokens: u64) -> Value {
    json!({"request_id": id, "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": tokens - 5, "output_tokens": 5})
}

/// `json.dumps` of the budget Python's `configure_usage_budget` writes.
fn budget_text(limit: u64, strict_unknown: bool, warned: bool) -> String {
    format!(r#"{{"token_limit": {limit}, "strict_unknown": {strict_unknown}, "warned": {warned}}}"#)
}

/// A budget one writer configured is the other writer's budget, byte for
/// byte (Python's `json.dumps` text): configuring the same budget again on
/// the other writes nothing (no second `usage-budget` event, the payload's
/// bytes unchanged), and the warning either marks re-dumps the payload the
/// other wrote.
#[test]
fn a_budget_either_side_configured_is_the_same_budget() {
    let board = Mixed::running();
    let other = board
        .other
        .call("parent", "usage_budget", &json!([100, false]), NOW + 3.0);
    assert_eq!(board.events("usage-budget"), "1");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([100, false]), NOW + 4.0);
    assert_eq!(other, rust, "the same budget, unchanged by the second");
    assert_eq!(board.events("usage-budget"), "1", "Rust wrote nothing");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    // Rust marks the warning on the budget the other wrote: 85 of 100.
    assert!(matches!(
        board.rust_records(&measured("r1", 85), 5.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.events("usage-warning"), "1");
    assert_eq!(board.budget_payload(), budget_text(100, false, true));

    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([200, true]), NOW + 6.0);
    assert_eq!(board.events("usage-budget"), "2");
    assert_eq!(board.budget_payload(), budget_text(200, true, false));
    let other = board
        .other
        .call("parent", "usage_budget", &json!([200, true]), NOW + 7.0);
    assert_eq!(other, rust);
    assert_eq!(
        board.events("usage-budget"),
        "2",
        "the other writer wrote nothing"
    );
    assert_eq!(board.budget_payload(), budget_text(200, true, false));
    // The other marks the warning on the budget Rust wrote: 165 of 200.
    assert!(matches!(
        board.other_records(&measured("r2", 80), 8.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.events("usage-warning"), "2");
    assert_eq!(board.budget_payload(), budget_text(200, true, true));
    assert_eq!(board.report(9.0)["budget"]["warned"], json!(true));
}

// ─── S13 (#2278): the harness's board beside other writers ───────────────

use crate::swarm_board_diff_runs::swarm_board_diff::dump::{Dump, first_difference, logical_dump};

/// Which writer serves a step of a mixed sequence: the one that drew every
/// id (the Python board until #2283), or the other.
#[derive(Clone, Copy, Debug)]
enum Side {
    Drawing,
    Rust,
}

/// One step: the side that serves it in the mixed run, the member, the
/// method and its arguments as JSON text. Every step that draws an id
/// (a run id, a reservation, a claim or file token) runs on the drawing
/// writer, so the mixed run draws the same ids, in the same order, as the
/// single-writer run.
struct Mix(Side, &'static str, &'static str, String);

fn mix(side: Side, member: &'static str, method: &'static str, args: Value) -> Mix {
    Mix(side, member, method, args.to_string())
}

/// `{token}` in a step's arguments is the token the claim answered.
fn with_token(args: &str, token: &str) -> String {
    args.replace("{token}", token)
}

/// An alternating sequence over every method group: membership, tasks and
/// claims, submissions, reservations, messages, evidence, usage, reads and
/// control.
fn alternating() -> Vec<Mix> {
    use Side::{Drawing, Rust};
    vec![
        mix(Rust, "parent", "_admit", json!(["worker", "res-w"])),
        mix(
            Drawing,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
        mix(
            Rust,
            "parent",
            "task_create",
            json!(["r1", "first", ["tests pass"]]),
        ),
        mix(
            Drawing,
            "parent",
            "task_create",
            json!(["r2", "second", ["docs"], [1]]),
        ),
        mix(Rust, "parent", "dependencies", json!([2, [1]])),
        mix(Drawing, "worker", "claim", json!([1])),
        mix(Rust, "worker", "task", json!([1])),
        mix(
            Rust,
            "worker",
            "block",
            json!([1, "{token}", "waiting on review"]),
        ),
        mix(
            Drawing,
            "worker",
            "unblock",
            json!([1, "{token}", "reviewed"]),
        ),
        mix(
            Drawing,
            "worker",
            "reserve",
            json!([1, "{token}", ["src/lib.rs"]]),
        ),
        mix(Rust, "worker", "file_owners", json!([])),
        mix(
            Rust,
            "worker",
            "send",
            json!(["m1", "parent", "ready for review"]),
        ),
        mix(Drawing, "parent", "inbox", json!([])),
        mix(Rust, "parent", "ack", json!([1])),
        mix(
            Drawing,
            "worker",
            "send",
            json!(["m2", "parent", "second note"]),
        ),
        mix(Rust, "worker", "withdraw", json!([2])),
        mix(
            Rust,
            "worker",
            "evidence",
            json!(["tests", "report", "R1", "command", true]),
        ),
        mix(
            Drawing,
            "worker",
            "submit",
            json!([1, "{token}", [{"artifact": "report", "revision": "R1"}]]),
        ),
        mix(Rust, "parent", "verify_task", json!([1, "{token}", "R1"])),
        mix(Drawing, "parent", "usage_budget", json!([1000, false])),
        mix(
            Rust,
            "worker",
            "_record_request",
            json!([{"request_id": "q1", "instrumented_attempts": 1, "outcome": "succeeded", "context_input_tokens": 40, "output_tokens": 5}]),
        ),
        mix(Drawing, "parent", "usage_report", json!([])),
        mix(Rust, "parent", "summary", json!([null])),
        mix(Drawing, "parent", "events", json!([0, 50])),
        mix(Rust, "parent", "pause", json!(["hold for review"])),
        mix(Drawing, "parent", "_control_status", json!([])),
        mix(Rust, "parent", "_resume_external", json!([])),
        mix(
            Drawing,
            "parent",
            "amend",
            json!(["mixed writers", ["keep the schema"], [{"id": "t", "kind": "command", "description": "test"}], "narrowed"]),
        ),
        mix(Rust, "parent", "tasks", json!([])),
        mix(Rust, "parent", "stop", json!(["cancelled", "done"])),
    ]
}

/// The claim token a claim answered.
fn claimed_token(outcome: &Outcome) -> Option<String> {
    match outcome {
        Outcome::Ok(task) => task["token"].as_str().map(str::to_owned),
        _ => None,
    }
}

/// The run `created` creates, as `Mixed::running` does (with `worker` not
/// yet admitted).
fn create_args() -> Value {
    json!({
        "goal": "mixed writers",
        "constraints": [],
        "criteria": [{"id": "tests", "kind": "command", "description": "test"}],
        "member_limit": 5,
        "deadline": NOW + 3_600.0,
    })
}

/// A fresh board file in `dir`, created by the drawing writer, which it
/// answers with.
fn created(dir: &Path, name: &str) -> (PathBuf, PathBuf, RustBoard) {
    let checkout = dir.join(name);
    std::fs::create_dir_all(&checkout).expect("create the board directory");
    let database = checkout.join("swarm.sqlite");
    let drawing = RustBoard::open(&database, &checkout);
    let outcome = drawing.call("parent", "create_run", &create_args(), NOW);
    assert!(matches!(outcome, Outcome::Ok(_)), "create_run: {outcome:?}");
    (checkout, database, drawing)
}

/// The single writer's run of `mixes` (tokens filled in as its claim
/// answered them), as golden steps: the creation first, then each call at
/// `NOW + 1 + index`. Its answers, run on one board, are compared with
/// Python's frozen answers (`run_golden`) before any mixed run is compared
/// with the single writer's.
fn single_writer_steps(mixes: &[Mix]) -> Vec<Step> {
    let dir = tempfile::tempdir().expect("a directory for the single writer");
    let (_, _, single) = created(dir.path(), "single");
    let mut steps = vec![step_text(
        "parent",
        "create_run",
        &create_args().to_string(),
        NOW,
    )];
    let mut token = String::new();
    for (index, Mix(_, member, method, args)) in mixes.iter().enumerate() {
        let now = NOW + 1.0 + index as f64;
        let args = with_token(args, &token);
        let answered = single.call(member, method, &serde_json::from_str(&args).unwrap(), now);
        if *method == "claim" {
            token = claimed_token(&answered).expect("the claim answered a token");
        }
        steps.push(step_text(member, method, &args, now));
    }
    steps
}

#[test]
fn interleaved_writers_on_one_file_match_a_single_writer() {
    run_golden(&single_writer_steps(&alternating()));
    let dir = tempfile::tempdir().expect("a directory for the boards");
    let (_, single_file, single) = created(dir.path(), "single");
    let (mixed_checkout, mixed_file, drawing) = created(dir.path(), "mixed");
    let rust = RustBoard::open(&mixed_file, &mixed_checkout);
    let (mut single_token, mut mixed_token) = (String::new(), String::new());
    let mut served = 0;
    for (index, Mix(side, member, method, args)) in alternating().into_iter().enumerate() {
        let now = NOW + 1.0 + index as f64;
        let expected = single.call(
            member,
            method,
            &serde_json::from_str(&with_token(&args, &single_token)).unwrap(),
            now,
        );
        let args: Value = serde_json::from_str(&with_token(&args, &mixed_token)).unwrap();
        let answered = match side {
            Side::Drawing => drawing.call(member, method, &args, now),
            Side::Rust => rust.call(member, method, &args, now),
        };
        assert_eq!(answered, expected, "step {index} {method} on {side:?}");
        if method == "claim" {
            single_token = claimed_token(&expected).expect("the claim answered a token");
            mixed_token = claimed_token(&answered).expect("the claim answered a token");
        }
        served += usize::from(matches!(answered, Outcome::Ok(_)));
    }
    assert!(
        served >= 25,
        "the sequence exercised the board: {served} served"
    );
    let (single, mixed): (Dump, Dump) = (logical_dump(&single_file), logical_dump(&mixed_file));
    assert_eq!(
        first_difference(&single, &mixed),
        None,
        "the mixed file is the single writer's file"
    );
}

/// The claimed-token sequence: every id drawn by the drawing writer but
/// the claim's.
fn claim_sequence() -> Vec<Mix> {
    use Side::{Drawing, Rust};
    vec![
        mix(Drawing, "parent", "_admit", json!(["worker", "res-w"])),
        mix(
            Drawing,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
        mix(
            Drawing,
            "parent",
            "task_create",
            json!(["r1", "first", ["tests pass"]]),
        ),
        mix(Rust, "worker", "claim", json!([1])),
        mix(Drawing, "worker", "block", json!([1, "{token}", "waiting"])),
        mix(Drawing, "worker", "unblock", json!([1, "{token}", "ready"])),
        mix(
            Drawing,
            "worker",
            "evidence",
            json!(["tests", "report", "R1", "command", true]),
        ),
        mix(
            Drawing,
            "worker",
            "submit",
            json!([1, "{token}", [{"artifact": "report", "revision": "R1"}]]),
        ),
        mix(Drawing, "parent", "task", json!([1])),
    ]
}

/// A claim token one writer draws is the one another's calls then use
/// (#2278 review L4): on a file the drawing writer created, a second board
/// claims (drawing the token from a counter at the first's count, so it
/// draws the id the first's claim would), and the first blocks, unblocks
/// and submits against that token. The outcomes and the file are the
/// single writer's, ids included, and the single writer's are Python's.
#[test]
fn a_claim_one_writer_draws_serves_another_as_its_own_claim_would() {
    run_golden(&single_writer_steps(&claim_sequence()));
    let dir = tempfile::tempdir().expect("a directory for the boards");
    let (_, single_file, single) = created(dir.path(), "single");
    let (mixed_checkout, mixed_file, drawing) = created(dir.path(), "mixed");
    let mut token = String::new();
    for (index, Mix(side, member, method, args)) in claim_sequence().into_iter().enumerate() {
        let now = NOW + 1.0 + index as f64;
        let args: Value = serde_json::from_str(&with_token(&args, &token)).unwrap();
        let expected = single.call(member, method, &args, now);
        let answered = match side {
            Side::Drawing => drawing.call(member, method, &args, now),
            Side::Rust => {
                // The drawing writer drew every id so far; the single
                // writer's claim is its next: the claimant's counter starts
                // where the drawing writer's is.
                let drawn = claimed_token(&expected)
                    .and_then(|token| u64::from_str_radix(&token, 16).ok())
                    .expect("the claim drew a counter id");
                RustBoard::open_after(&mixed_file, &mixed_checkout, drawn - 1)
                    .call(member, method, &args, now)
            }
        };
        assert_eq!(answered, expected, "step {index} {method} on {side:?}");
        assert!(
            matches!(answered, Outcome::Ok(_)),
            "step {index} {method}: {answered:?}"
        );
        if method == "claim" {
            token = claimed_token(&answered).expect("the claim answered a token");
        }
    }
    assert_eq!(
        first_difference(&logical_dump(&single_file), &logical_dump(&mixed_file)),
        None,
        "the claimant's token, used by the other writer, leaves the single writer's file"
    );
}

/// Retries `call` while the store answers the contended refusal, within a
/// bound, counting each retry in `retries`: 25 appends against 7 other
/// writers each wait at most the 500 ms busy timeout per attempt.
fn until_uncontended(
    retries: &std::sync::atomic::AtomicUsize,
    mut call: impl FnMut() -> Outcome,
) -> Outcome {
    const CONTENDED: &str = "coordination store unavailable or contended: database is locked";
    for _ in 0..400 {
        match call() {
            Outcome::Refused(text) if text == CONTENDED => {
                retries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            answered => return answered,
        }
    }
    panic!("still contended after 400 attempts")
}

/// Eight writers, each its own board handles, append to one file at once
/// (review L5; half of them were Python writers until #2283): they start
/// together, while the test holds the file's write lock past the 500 ms
/// busy timeout, so the writers are refused as contended and retry.
/// Nothing is lost or duplicated: 200 distinct tasks and 200 distinct
/// request ids, and the file passes `integrity_check`.
#[test]
fn concurrent_writers_contend_without_corrupting_the_board() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    const WRITERS: usize = 8;
    let board = Mixed::running();
    let checkout = board.database.parent().unwrap().to_path_buf();
    let database = board.database.clone();
    let start = Arc::new(Barrier::new(WRITERS + 1));
    let retries = Arc::new(AtomicUsize::new(0));
    let workers: Vec<std::thread::JoinHandle<()>> = (0..WRITERS)
        .map(|writer| {
            let (database, checkout) = (database.clone(), checkout.clone());
            let (start, retries) = (start.clone(), retries.clone());
            std::thread::spawn(move || {
                let rust = RustBoard::open(&database, &checkout);
                start.wait();
                for task in 0..25 {
                    let args = json!([
                        format!("w{writer}-{task}"),
                        format!("writer {writer} task {task}"),
                        ["done"]
                    ]);
                    let now = NOW + 10.0 + f64::from(task);
                    let answered = until_uncontended(&retries, || {
                        rust.call("parent", "task_create", &args, now)
                    });
                    assert!(
                        matches!(answered, Outcome::Ok(_)),
                        "writer {writer}: {answered:?}"
                    );
                }
            })
        })
        .collect();
    // Every writer's first append meets the lock held past its timeout.
    let holder = Connection::open(&database).expect("open the shared board");
    holder
        .execute_batch("BEGIN IMMEDIATE")
        .expect("take the write lock");
    start.wait();
    std::thread::sleep(std::time::Duration::from_millis(1_200));
    holder
        .execute_batch("COMMIT")
        .expect("release the write lock");
    for worker in workers {
        worker.join().expect("a writer finished");
    }
    assert!(
        retries.load(Ordering::SeqCst) >= 1,
        "the writers met the held lock, were refused as contended and retried: {} retries",
        retries.load(Ordering::SeqCst)
    );
    let count = |sql: &str| board.stored_text(sql);
    assert_eq!(count("SELECT CAST(count(*) AS TEXT) FROM tasks"), "200");
    assert_eq!(
        count("SELECT CAST(count(DISTINCT title) AS TEXT) FROM tasks"),
        "200",
        "every append is its own task"
    );
    assert_eq!(
        count("SELECT CAST(count(DISTINCT request) AS TEXT) FROM requests WHERE request LIKE 'w%'"),
        "200",
        "every append's request id recorded once"
    );
    assert_eq!(board.events("task_created"), "200");
    assert_eq!(count("PRAGMA integrity_check"), "ok");
}
