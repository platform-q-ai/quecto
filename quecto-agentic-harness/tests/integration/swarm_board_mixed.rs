//! Mixed writers on one board file (#2274; S13 extends it): the Python
//! board and the Rust board take turns on the **same** SQLite file, as a
//! swarm whose members run either implementation would. A redelivery is
//! compared by decoded value (Python's `==`), so these tests also read the
//! raw rows: each side must store the very bytes the other side's `encode`
//! (or, for the budget, `json.dumps`) writes, and neither may write where
//! the other would not.
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;

/// One board file both implementations open.
struct Mixed {
    _dir: tempfile::TempDir,
    database: PathBuf,
    python: PyBoard,
    rust: RustBoard,
}

impl Mixed {
    /// A running run coordinated by `parent` (created by Python), with
    /// `worker` live.
    fn running() -> Self {
        let dir = tempfile::tempdir().expect("a directory for the board");
        let checkout = dir.path().join("board");
        std::fs::create_dir_all(&checkout).expect("create the board directory");
        let database = checkout.join("swarm.sqlite");
        let mut python = PyBoard::start(&database, &checkout, dir.path());
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
            let outcome = python.call("parent", method, &args.to_string(), NOW + offset);
            assert!(matches!(outcome, Outcome::Ok(_)), "{method}: {outcome:?}");
        }
        assert_board(&database);
        Self {
            _dir: dir,
            database,
            python,
            rust,
        }
    }

    fn python_records(&mut self, record: &Value, offset: f64) -> Outcome {
        let args = json!([record]).to_string();
        self.python
            .call("worker", "_record_request", &args, NOW + offset)
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

    /// The report both sides read of the one file: identical.
    fn report(&mut self, offset: f64) -> Value {
        let python = self
            .python
            .call("parent", "usage_report", "[]", NOW + offset);
        let rust = self
            .rust
            .call("parent", "usage_report", &json!([]), NOW + offset);
        assert_eq!(python, rust, "both read the same report");
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

#[test]
fn a_python_recorded_request_redelivered_by_rust_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(
        board.python_records(&pending, 3.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.request_payload(), PENDING, "Python's encode");
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
    assert!(matches!(board.python_records(&known, 8.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Python rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.rust_records(&changed, 9.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.request_payload(), KNOWN);
}

#[test]
fn a_rust_recorded_request_redelivered_by_python_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(board.rust_records(&pending, 3.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "Rust's encode, Python's bytes"
    );
    assert!(matches!(
        board.python_records(&pending, 4.0),
        Outcome::Ok(_)
    ));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "no digest: nothing written"
    );
    assert_eq!(board.report(5.0)["totals"]["requests"], json!(1));
    assert!(matches!(board.python_records(&known, 6.0), Outcome::Ok(_)));
    assert_eq!(board.request_payload(), KNOWN, "Python's encode");
    assert!(matches!(board.rust_records(&known, 7.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Rust rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.python_records(&changed, 8.0),
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

/// A budget one side configured is the other side's budget, byte for byte:
/// configuring the same budget again on the other side writes nothing (no
/// second `usage-budget` event, the payload's bytes unchanged), and the
/// warning either side marks re-dumps the payload the other side wrote.
#[test]
fn a_budget_either_side_configured_is_the_same_budget() {
    let mut board = Mixed::running();
    let python = board
        .python
        .call("parent", "usage_budget", "[100, false]", NOW + 3.0);
    assert_eq!(board.events("usage-budget"), "1");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([100, false]), NOW + 4.0);
    assert_eq!(python, rust, "the same budget, unchanged by the second");
    assert_eq!(board.events("usage-budget"), "1", "Rust wrote nothing");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    // Rust marks the warning on the budget Python wrote: 85 of 100.
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
    let python = board
        .python
        .call("parent", "usage_budget", "[200, true]", NOW + 7.0);
    assert_eq!(python, rust);
    assert_eq!(board.events("usage-budget"), "2", "Python wrote nothing");
    assert_eq!(board.budget_payload(), budget_text(200, true, false));
    // Python marks the warning on the budget Rust wrote: 165 of 200.
    assert!(matches!(
        board.python_records(&measured("r2", 80), 8.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.events("usage-warning"), "2");
    assert_eq!(board.budget_payload(), budget_text(200, true, true));
    assert_eq!(board.report(9.0)["budget"]["warned"], json!(true));
}

// ─── S13 (#2278): the harness's Rust board beside Python writers ─────────

use crate::swarm_board_diff_runs::swarm_board_diff::dump::{Dump, first_difference, logical_dump};

/// Which implementation a step of a mixed sequence runs on.
#[derive(Clone, Copy, Debug)]
enum Side {
    Python,
    Rust,
}

/// One step: the side that serves it in the mixed run, the member, the
/// method and its arguments as JSON text. Every step that draws an id
/// (a run id, a reservation, a claim or file token) runs on Python, so
/// the mixed run draws the same ids, in the same order, as the
/// Python-only run.
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
    use Side::{Python, Rust};
    vec![
        mix(Rust, "parent", "_admit", json!(["worker", "res-w"])),
        mix(
            Python,
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
            Python,
            "parent",
            "task_create",
            json!(["r2", "second", ["docs"], [1]]),
        ),
        mix(Rust, "parent", "dependencies", json!([2, [1]])),
        mix(Python, "worker", "claim", json!([1])),
        mix(Rust, "worker", "task", json!([1])),
        mix(
            Rust,
            "worker",
            "block",
            json!([1, "{token}", "waiting on review"]),
        ),
        mix(
            Python,
            "worker",
            "unblock",
            json!([1, "{token}", "reviewed"]),
        ),
        mix(
            Python,
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
        mix(Python, "parent", "inbox", json!([])),
        mix(Rust, "parent", "ack", json!([1])),
        mix(
            Python,
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
            Python,
            "worker",
            "submit",
            json!([1, "{token}", [{"artifact": "report", "revision": "R1"}]]),
        ),
        mix(Rust, "parent", "verify_task", json!([1, "{token}", "R1"])),
        mix(Python, "parent", "usage_budget", json!([1000, false])),
        mix(
            Rust,
            "worker",
            "_record_request",
            json!([{"request_id": "q1", "instrumented_attempts": 1, "outcome": "succeeded", "context_input_tokens": 40, "output_tokens": 5}]),
        ),
        mix(Python, "parent", "usage_report", json!([])),
        mix(Rust, "parent", "summary", json!([null])),
        mix(Python, "parent", "events", json!([0, 50])),
        mix(Rust, "parent", "pause", json!(["hold for review"])),
        mix(Python, "parent", "_control_status", json!([])),
        mix(Rust, "parent", "_resume_external", json!([])),
        mix(
            Python,
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

/// A fresh board file in `dir`, created by Python as `Mixed::running` does
/// (with `worker` not yet admitted).
fn created(dir: &Path, name: &str) -> (PathBuf, PathBuf, PyBoard) {
    let checkout = dir.join(name);
    std::fs::create_dir_all(&checkout).expect("create the board directory");
    let database = checkout.join("swarm.sqlite");
    let mut python = PyBoard::start(&database, &checkout, &checkout);
    let create = json!({
        "goal": "mixed writers",
        "constraints": [],
        "criteria": [{"id": "tests", "kind": "command", "description": "test"}],
        "member_limit": 5,
        "deadline": NOW + 3_600.0,
    });
    let outcome = python.call("parent", "create_run", &create.to_string(), NOW);
    assert!(matches!(outcome, Outcome::Ok(_)), "create_run: {outcome:?}");
    (checkout, database, python)
}

#[test]
fn interleaved_python_and_rust_writers_on_one_file_match_a_single_writer() {
    let dir = tempfile::tempdir().expect("a directory for the boards");
    let (_, single_file, mut single) = created(dir.path(), "single");
    let (mixed_checkout, mixed_file, mut python) = created(dir.path(), "mixed");
    let rust = RustBoard::open(&mixed_file, &mixed_checkout);
    let (mut single_token, mut mixed_token) = (String::new(), String::new());
    let mut served = 0;
    for (index, Mix(side, member, method, args)) in alternating().into_iter().enumerate() {
        let now = NOW + 1.0 + index as f64;
        let expected = single.call(member, method, &with_token(&args, &single_token), now);
        let args = with_token(&args, &mixed_token);
        let answered = match side {
            Side::Python => python.call(member, method, &args, now),
            Side::Rust => rust.call(member, method, &serde_json::from_str(&args).unwrap(), now),
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

/// Retries `call` while the store answers the contended refusal, within a
/// bound: 25 appends against 7 other writers each wait at most the 500 ms
/// busy timeout per attempt.
fn until_uncontended(mut call: impl FnMut() -> Outcome) -> Outcome {
    const CONTENDED: &str = "coordination store unavailable or contended: database is locked";
    for _ in 0..400 {
        match call() {
            Outcome::Refused(text) if text == CONTENDED => continue,
            answered => return answered,
        }
    }
    panic!("still contended after 400 attempts")
}

#[test]
fn python_and_rust_contend_without_corrupting_the_board() {
    let board = Mixed::running();
    let checkout = board.database.parent().unwrap().to_path_buf();
    let database = board.database.clone();
    let workers: Vec<std::thread::JoinHandle<()>> = (0..8)
        .map(|writer| {
            let (database, checkout) = (database.clone(), checkout.clone());
            std::thread::spawn(move || {
                let scratch = tempfile::tempdir().expect("a directory for the driver");
                let mut python =
                    (writer % 2 == 0).then(|| PyBoard::start(&database, &checkout, scratch.path()));
                let rust = RustBoard::open(&database, &checkout);
                for task in 0..25 {
                    let args = json!([
                        format!("w{writer}-{task}"),
                        format!("task {task}"),
                        ["done"]
                    ]);
                    let now = NOW + 10.0 + f64::from(task);
                    let answered = until_uncontended(|| match python.as_mut() {
                        Some(python) => {
                            python.call("parent", "task_create", &args.to_string(), now)
                        }
                        None => rust.call("parent", "task_create", &args, now),
                    });
                    assert!(
                        matches!(answered, Outcome::Ok(_)),
                        "writer {writer}: {answered:?}"
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("a writer finished");
    }
    let count = |sql: &str| board.stored_text(sql);
    assert_eq!(count("SELECT CAST(count(*) AS TEXT) FROM tasks"), "200");
    assert_eq!(
        count("SELECT CAST(count(DISTINCT id) AS TEXT) FROM tasks"),
        "200"
    );
    assert_eq!(board.events("task_created"), "200");
    assert_eq!(count("PRAGMA integrity_check"), "ok");
}

/// A member's `op=run` program on the real `SwarmTool`, as `context`'s
/// member: it must succeed.
async fn run_program(
    context: &quecto::infrastructure::tools::swarm_bridge::SwarmContext,
    code: &str,
) {
    use quecto::application::tools::ports::Tool;
    use quecto::infrastructure::security::sandbox::Sandbox;
    use quecto::infrastructure::tools::swarm::{SwarmConfig, SwarmTool};
    let workspace = std::sync::Arc::new(context.checkout.clone());
    let tool = SwarmTool::new(
        workspace.clone(),
        std::sync::Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig::default(),
    )
    .with_context(Some(context.clone()));
    let result = tool
        .execute(&json!({"op": "run", "code": code}).to_string())
        .await
        .expect("the program ran");
    assert!(!result.is_error, "{code}: {}", result.content);
}

/// A dump with what differs between two real runs masked: every REAL
/// (times, deadlines), and in text every 32-hex id (run ids,
/// reservations, tokens; numbered by first appearance, so the same id
/// stays the same) and every decimal fraction (times inside JSON).
fn masked(dump: Dump) -> Dump {
    use rusqlite::types::Value as SqlValue;
    let mut ids: Vec<String> = Vec::new();
    let mut mask_text = |text: &str| -> String {
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let hex = chars[index..]
                .iter()
                .take_while(|c| c.is_ascii_digit() || ('a'..='f').contains(*c))
                .count();
            let digits = chars[index..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let fraction = digits > 0
                && chars.get(index + digits) == Some(&'.')
                && chars
                    .get(index + digits + 1)
                    .is_some_and(char::is_ascii_digit);
            if hex == 32 {
                let id: String = chars[index..index + 32].iter().collect();
                let number = ids.iter().position(|seen| *seen == id).unwrap_or_else(|| {
                    ids.push(id);
                    ids.len() - 1
                });
                out.push_str(&format!("<id{number}>"));
                index += 32;
            } else if fraction {
                let tail = chars[index + digits + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                    .count();
                out.push_str("<real>");
                index += digits + 1 + tail;
            } else {
                let run = hex.max(1);
                out.extend(&chars[index..index + run]);
                index += run;
            }
        }
        out
    };
    dump.into_iter()
        .map(|(table, rows)| {
            let rows = rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|(kind, value)| match value {
                            SqlValue::Real(_) => (kind, SqlValue::Real(0.0)),
                            SqlValue::Text(text) => (kind, SqlValue::Text(mask_text(&text))),
                            other => (kind, other),
                        })
                        .collect()
                })
                .collect();
            (table, rows)
        })
        .collect()
}

#[tokio::test]
async fn python_member_programs_and_rust_harness_share_one_board() {
    use quecto::application::swarm::ports::CoordinationPort;
    use quecto::domain::swarm::{MemberExit, ProcessIdentity};
    use quecto::infrastructure::tools::swarm_bridge::{SwarmContext, process_start};
    let root = tempfile::tempdir().expect("a directory for the boards");
    let checkout = root
        .path()
        .join(".quecto/container-environments/environment/workspace/repo");
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = |member: &str| SwarmContext {
        checkout: checkout.clone(),
        member: member.into(),
        lifecycle: std::sync::Arc::new(quecto::application::swarm::LifecycleService),
        board: quecto::composition::swarm::swarm_board(),
    };
    let (coordinator, worker) = (context("parent"), context("worker"));
    let identity = ProcessIdentity {
        pid: std::process::id(),
        started: process_start(std::process::id()).unwrap(),
    };
    // Real time: the tool and the harness read the wall clock.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let deadline = (now + 3_600.0).floor();
    let contract = json!({"goal": "mixed", "constraints": [], "criteria": [{"id": "tests", "kind": "command", "description": "pass"}], "member_limit": 3, "deadline": deadline});
    // The harness (Rust) creates the run and admits the worker.
    coordinator.create_run(&contract, &identity, None).unwrap();
    coordinator.reserve_member("worker", "res-w").unwrap();
    worker.join(&identity, None, Some("res-w")).unwrap();
    // 1. A Python member program creates and claims a task.
    run_program(&worker, "from swarm import board; t = board.task_create('r1', 'first', ['tests pass']); board.claim(t['id'])").await;
    // 2. The harness's Rust summary sees it.
    let summary = tokio::task::spawn_blocking({
        let coordinator = coordinator.clone();
        move || coordinator.summary()
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(summary["tasks"][0]["status"], "claimed", "{summary}");
    assert_eq!(summary["tasks"][0]["owner"], "worker");
    // 3. The harness's Rust `_confirmed_dead` blocks it.
    coordinator
        .confirm_dead("worker", MemberExit::Orderly)
        .unwrap();
    // 4. A Python `recover` reopens it.
    run_program(&coordinator, "from swarm import board; board.recover(1)").await;
    let summary = coordinator.summary().unwrap();
    assert_eq!(summary["tasks"][0]["status"], "ready", "{summary}");

    // 5. The same sequence on the pure-Python board: every call the
    // harness and the tool made above, in order.
    let replay = root.path().join("replay");
    std::fs::create_dir_all(&replay).unwrap();
    let database = replay.join("swarm.sqlite");
    let mut python = PyBoard::start(&database, &replay, &replay);
    let created = python.call(
        "parent",
        "create",
        &json!(["mixed", [], contract["criteria"], 3, deadline]).to_string(),
        now,
    );
    let Outcome::Ok(created) = created else {
        panic!("create: {created:?}")
    };
    let reservation = created["members"]
        .as_array()
        .and_then(|members| members.iter().find(|m| m["id"] == "parent"))
        .and_then(|m| m["reservation"].as_str())
        .unwrap()
        .to_owned();
    let pid = identity.pid;
    let started = identity.started.clone();
    let replayed: Vec<(&str, &str, Value)> = vec![
        (
            "parent",
            "_activate",
            json!(["parent", reservation, pid, started, null]),
        ),
        ("parent", "_snapshot", json!([])),
        ("parent", "_admit", json!(["worker", "res-w"])),
        ("worker", "_bootstrap", json!([pid, started, null, "res-w"])),
        // The worker's op=run: the tool's summary, the program, the tool's
        // summary, and its notifications (the event cursor moved).
        ("worker", "summary", json!([null])),
        (
            "worker",
            "task_create",
            json!(["r1", "first", ["tests pass"]]),
        ),
        ("worker", "claim", json!([1])),
        ("worker", "summary", json!([null])),
        ("worker", "_notifications", json!([true])),
        ("parent", "summary", json!([null])),
        ("parent", "_confirmed_dead", json!(["worker", "orderly"])),
        ("parent", "summary", json!([null])),
        ("parent", "recover", json!([1])),
        ("parent", "summary", json!([null])),
        ("parent", "_notifications", json!([true])),
        ("parent", "summary", json!([null])),
    ];
    for (member, method, args) in replayed {
        let outcome = python.call(member, method, &args.to_string(), now);
        assert!(
            matches!(outcome, Outcome::Ok(_)),
            "{member} {method}: {outcome:?}"
        );
    }
    let harness = masked(logical_dump(&coordinator.database()));
    let pure = masked(logical_dump(&database));
    assert_eq!(
        first_difference(&pure, &harness),
        None,
        "the harness's Rust calls and Python member programs leave the pure-Python board"
    );
}
