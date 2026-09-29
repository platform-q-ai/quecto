//! #2279: the structured board ops through `SwarmTool::execute`, each on a
//! real temp board: the same answer the board method gives, the calling
//! syntax's own refusals, the running gate, and `op=run`'s post-call
//! lifecycle (wake hints, settlement).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::application::swarm::ports::{
    CoordinationPort, PortFuture, ProcessControl, ProcessObservation, SettlementStep,
    SwarmLifecycle,
};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::swarm::{BoardOpObservation, MemberExit, ProcessIdentity, Snapshot};
use crate::domain::tool::ToolResult;
use crate::infrastructure::tools::swarm::SwarmTool;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// The real lifecycle, counting its settlements; its settlement fails
/// while `fail_settle` is set.
#[derive(Debug, Default)]
pub(super) struct CountingLifecycle {
    pub(super) settled: AtomicUsize,
    pub(super) fail_settle: std::sync::atomic::AtomicBool,
}

impl SwarmLifecycle for CountingLifecycle {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError> {
        crate::application::swarm::LifecycleService.reconcile(coordination, processes)
    }
    fn member_exited(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
        member: &str,
        exit: MemberExit,
    ) -> Result<Snapshot, DomainError> {
        crate::application::swarm::LifecycleService.member_exited(
            coordination,
            processes,
            member,
            exit,
        )
    }
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
        observation: &'a (dyn ProcessObservation + Sync),
    ) -> PortFuture<'a, Result<(), DomainError>> {
        self.settled.fetch_add(1, Ordering::SeqCst);
        match self.fail_settle.load(Ordering::SeqCst) {
            true => Box::pin(async { Err(DomainError::Tool("settlement unavailable".into())) }),
            false => crate::application::swarm::LifecycleService.settle(
                snapshot,
                actor,
                processes,
                observation,
            ),
        }
    }
    fn settlement_step(
        &self,
        snapshot: &Snapshot,
        actor: &str,
        elapsed: std::time::Duration,
        grace: std::time::Duration,
    ) -> SettlementStep {
        crate::application::swarm::LifecycleService.settlement_step(snapshot, actor, elapsed, grace)
    }
    fn settle_overdue<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> PortFuture<'a, Result<(), DomainError>> {
        crate::application::swarm::LifecycleService.settle_overdue(snapshot, actor, processes)
    }
    fn observed_outcome(
        &self,
        snapshot: &Snapshot,
        clock: &dyn crate::application::swarm::ports::Clock,
    ) -> crate::domain::swarm::RunStatus {
        crate::application::swarm::LifecycleService.observed_outcome(snapshot, clock)
    }
}

/// The event log, in memory.
#[derive(Default)]
pub(super) struct Recorded(Mutex<Vec<BoardOpObservation>>);

impl crate::application::swarm::ports::BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }

    /// No summary this test checks is written.
    fn summarize(&self, _summary: crate::domain::swarm::SwarmRunSummary) {}
}

impl Recorded {
    /// The ops recorded since the last take, by name, taken.
    pub(super) fn take(&self) -> Vec<BoardOpObservation> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// A coordinator's board in its own directory, its run created with
/// `deadline` (so two boards built alike hold the same run), over
/// `lifecycle`.
pub(super) fn board_with(
    deadline: u64,
    lifecycle: Arc<dyn SwarmLifecycle>,
) -> (tempfile::TempDir, SwarmContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle,
    };
    direct(
        &context,
        "create",
        json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 5, deadline]),
    )
    .unwrap();
    (directory, context)
}

/// A direct board call by `context`, made off the async workers as the
/// board's debug assertion requires of every caller (#2278 review L6).
pub(super) fn direct(
    context: &SwarmContext,
    method: &str,
    args: Value,
) -> Result<Value, DomainError> {
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.call(method, args))
}

pub(super) fn deadline() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600
}

pub(super) fn board() -> (tempfile::TempDir, SwarmContext) {
    board_with(
        deadline(),
        Arc::new(crate::application::swarm::LifecycleService),
    )
}

pub(super) fn tool(context: &SwarmContext) -> SwarmTool {
    SwarmTool::new().with_context(Some(context.clone()))
}

pub(super) async fn execute(context: &SwarmContext, arguments: &str) -> ToolResult {
    tool(context).execute(arguments).await.unwrap()
}

/// The answer of a successful op, parsed.
pub(super) async fn answered(context: &SwarmContext, arguments: Value) -> Value {
    let result = execute(context, &arguments.to_string()).await;
    assert!(!result.is_error, "{arguments}: {}", result.content);
    serde_json::from_str(&result.content)
        .unwrap_or_else(|error| panic!("{arguments}: {error}: {}", result.content))
}

/// The refusal text of an op.
pub(super) async fn refused(context: &SwarmContext, arguments: &str) -> String {
    let result = execute(context, arguments).await;
    assert!(result.is_error, "{arguments}: {}", result.content);
    result.content
}

/// A task the coordinator created, and its claim's token.
fn claimed(context: &SwarmContext) -> Value {
    direct(context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    direct(context, "claim", json!([1])).unwrap()["token"].clone()
}

fn submitted(context: &SwarmContext) -> Value {
    let token = claimed(context);
    direct(
        context,
        "submit",
        json!([1, token, [{"artifact": "a", "revision": "R1"}]]),
    )
    .unwrap();
    token
}

/// Every op of the mapping table (#2265 D4), after the setup it needs on a
/// coordinator's board, and its named arguments. The `match` in
/// [`every_mapped_op_returns_exactly_what_the_board_method_returns`] walks
/// `BOARD_OPS`, so an op added there fails until it is listed here.
fn prepared(op: &str, context: &SwarmContext) -> Value {
    match op {
        "task" => {
            claimed(context);
            json!({"task_id": 1})
        }
        "tasks" => {
            claimed(context);
            json!({"offset": 0, "limit": 10})
        }
        "file_owners" => {
            let token = claimed(context);
            direct(context, "reserve", json!([1, token, ["a.rs"]])).unwrap();
            json!({})
        }
        "task_create" => json!({"request": "r1", "title": "t", "acceptance": ["pass"]}),
        "dependencies" => {
            direct(context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
            direct(context, "task_create", json!(["r2", "b", ["pass"]])).unwrap();
            json!({"task_id": 2, "dependencies": [1]})
        }
        "claim" => {
            direct(context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
            json!({"task_id": 1})
        }
        "release" => json!({"task_id": 1, "token": claimed(context)}),
        "block" => json!({"task_id": 1, "token": claimed(context), "reason": "waiting"}),
        "unblock" => {
            let token = claimed(context);
            direct(context, "block", json!([1, token, "waiting"])).unwrap();
            json!({"task_id": 1, "token": token, "reason": "resolved"})
        }
        "submit" => json!({"task_id": 1, "token": claimed(context),
            "evidence": [{"artifact": "a", "revision": "R1"}]}),
        "reserve" => json!({"task_id": 1, "token": claimed(context), "paths": ["a.rs"]}),
        "release_files" => {
            let token = claimed(context);
            let reserved = direct(context, "reserve", json!([1, token, ["a.rs"]])).unwrap();
            json!({"task_id": 1, "token": token, "reservation": reserved["token"]})
        }
        "send" => json!({"request": "m1", "recipient": "coordinator", "body": "hi",
            "revision": "R1"}),
        "withdraw" | "ack" => {
            direct(context, "send", json!(["m1", "coordinator", "hi"])).unwrap();
            json!({"message_id": 1})
        }
        "inbox" => {
            direct(context, "send", json!(["m1", "coordinator", "hi"])).unwrap();
            json!({"include_consumed": true})
        }
        "evidence" => json!({"criterion": "tests", "artifact": "a", "revision": "R1",
            "kind": "command", "passed": true}),
        "amend" => json!({"goal": "g", "constraints": ["c"],
            "criteria": [{"id": "tests", "kind": "review", "description": "d"}],
            "reason": "why"}),
        "verify_task" => json!({"task_id": 1, "token": submitted(context), "revision": "R1"}),
        "revalidate_task" => {
            let token = submitted(context);
            direct(context, "verify_task", json!([1, token, "R1"])).unwrap();
            json!({"task_id": 1, "revision": "R2",
                "evidence": [{"artifact": "b", "revision": "R2"}]})
        }
        // The coordinator owns the claim and is alive: the board refuses
        // the recovery, and the refusal is the answer compared.
        "recover" => {
            claimed(context);
            json!({"task_id": 1, "release_files": true})
        }
        "revoke" => {
            claimed(context);
            json!({"task_id": 1, "reason": "reassign"})
        }
        "complete" => {
            direct(
                context,
                "evidence",
                json!(["tests", "a", "R1", "command", true]),
            )
            .unwrap();
            json!({"revision": "R1"})
        }
        "stop" => json!({"status": "blocked", "reason": "why"}),
        "usage_report" => json!({}),
        other => panic!("no setup for the mapped op {other}"),
    }
}

/// A board's answer with what two boards draw differently masked: ids
/// and tokens (32 hex digits) and times (floats).
fn masked(value: Value) -> Value {
    match value {
        Value::String(text) if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Value::from("<id>")
        }
        Value::Number(number) if number.is_f64() => Value::from("<time>"),
        Value::Array(items) => Value::Array(items.into_iter().map(masked).collect()),
        Value::Object(fields) => fields
            .into_iter()
            .map(|(key, value)| (key, masked(value)))
            .collect(),
        other => other,
    }
}

/// The op through the tool on one board, the board method through
/// `SwarmContext::call` on an identical one: equal answers, or equal
/// refusal texts.
#[tokio::test]
async fn every_mapped_op_returns_exactly_what_the_board_method_returns() {
    let ops = super::BOARD_OPS;
    assert_eq!(ops.len(), 25, "the epic's mapping table has 25 new ops");
    for spec in ops {
        let deadline = deadline();
        let lifecycle = || Arc::new(crate::application::swarm::LifecycleService);
        let (_by_op, by_op) = board_with(deadline, lifecycle());
        let (_by_method, by_method) = board_with(deadline, lifecycle());
        // Each board draws its own tokens: the arguments name each one's.
        let args = prepared(spec.name, &by_op);
        let method_args = prepared(spec.name, &by_method);
        assert_eq!(
            masked(args.clone()),
            masked(method_args.clone()),
            "{}",
            spec.name
        );
        let mut request = args;
        request["op"] = json!(spec.name);
        let result = execute(&by_op, &request.to_string()).await;
        match direct(&by_method, spec.name, method_args) {
            Ok(expected) => {
                assert!(!result.is_error, "{}: {}", spec.name, result.content);
                let answer: Value = serde_json::from_str(&result.content).unwrap();
                assert_eq!(masked(answer), masked(expected), "{}", spec.name);
            }
            Err(refusal) => {
                assert!(result.is_error, "{}: {}", spec.name, result.content);
                assert_eq!(result.content, refusal.to_string(), "{}", spec.name);
            }
        }
    }
}

#[tokio::test]
async fn claim_by_op_is_claim_by_board() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let claim = answered(&context, json!({"op": "claim", "task_id": 1})).await;
    let token = claim["token"]
        .as_str()
        .expect("the claim carries its token");
    assert_eq!(token.len(), 32, "{claim}");
    assert_eq!(claim["status"], "claimed", "{claim}");
    let events =
        crate::infrastructure::tools::call_work::off_the_runtime(|| context.events(0, 100))
            .unwrap();
    let claimed: Vec<&Value> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["action"] == "claimed")
        .collect();
    assert_eq!(claimed.len(), 1, "{events}");
    // The token works for the board's next call.
    direct(&context, "release", json!([1, token])).unwrap();
}

#[tokio::test]
async fn a_missing_argument_names_the_op_and_field() {
    let (_directory, context) = board();
    let text = refused(&context, r#"{"op":"claim"}"#).await;
    assert!(
        text.contains("claim: missing required argument task_id"),
        "{text}"
    );
    let text = refused(&context, r#"{"op":"release","task_id":1}"#).await;
    assert!(
        text.contains("release: missing required argument token"),
        "{text}"
    );
}

#[tokio::test]
async fn an_unexpected_argument_is_refused() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let text = refused(&context, r#"{"op":"claim","task_id":1,"owner":"me"}"#).await;
    assert!(text.contains("claim: unexpected argument owner"), "{text}");
    // Nothing was claimed.
    assert_eq!(
        direct(&context, "task", json!([1])).unwrap()["status"],
        "ready"
    );
}

/// Inherited behaviour (epic P3): Python binds `claim(True)` as task 1 and
/// claims it; the structured op does the same.
#[tokio::test]
async fn a_boolean_task_id_binds_as_python_binds_true() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let claim = answered(&context, json!({"op": "claim", "task_id": true})).await;
    assert_eq!(claim["id"], 1, "{claim}");
    assert_eq!(
        direct(&context, "task", json!([1])).unwrap()["status"],
        "claimed"
    );
}

/// `evidence`, `dependencies` and `release_files` are each an op and a
/// field of another op: each binds to the op named.
#[tokio::test]
async fn clashing_field_names_bind_to_the_op_named() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "a", ["pass"]])).unwrap();
    answered(
        &context,
        json!({"op": "task_create", "request": "r2", "title": "b", "acceptance": ["pass"],
            "dependencies": [1]}),
    )
    .await;
    assert_eq!(
        direct(&context, "task", json!([2])).unwrap()["dependencies"],
        json!([1])
    );
    answered(
        &context,
        json!({"op": "dependencies", "task_id": 2, "dependencies": []}),
    )
    .await;
    assert_eq!(
        direct(&context, "task", json!([2])).unwrap()["dependencies"],
        json!([])
    );
    let token = answered(&context, json!({"op": "claim", "task_id": 1})).await["token"].clone();
    answered(
        &context,
        json!({"op": "submit", "task_id": 1, "token": token,
            "evidence": [{"artifact": "a", "revision": "R1"}]}),
    )
    .await;
    assert_eq!(
        direct(&context, "task", json!([1])).unwrap()["evidence"],
        json!([{"artifact": "a", "revision": "R1"}])
    );
    answered(
        &context,
        json!({"op": "evidence", "criterion": "tests", "artifact": "a", "revision": "R1",
            "kind": "command", "passed": true}),
    )
    .await;
    let text = refused(
        &context,
        r#"{"op":"recover","task_id":1,"release_files":true}"#,
    )
    .await;
    // Bound to `recover` (its flag, not the op of that name): the board
    // answers the recovery's own refusal for a live owner's submitted task.
    assert!(
        text.contains("recovery requires confirmed worker death"),
        "{text}"
    );
    let text = refused(&context, r#"{"op":"release_files","task_id":1}"#).await;
    assert!(
        text.contains("release_files: missing required argument token"),
        "{text}"
    );
}

/// A worker whose endpoint refuses every wake hint.
pub(super) fn rejecting_worker(context: &SwarmContext) -> std::thread::JoinHandle<()> {
    use std::io::Write;
    let socket = context.checkout.join("worker.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let worker = SwarmContext {
        member: "worker".into(),
        ..context.clone()
    };
    // Board calls, made off the async workers (#2278 review L6).
    crate::infrastructure::tools::call_work::off_the_runtime(|| {
        context.reserve_member("worker", "reservation").unwrap();
        worker
            .join(
                &ProcessIdentity {
                    pid: 123,
                    started: "identity".into(),
                },
                socket.to_str(),
                Some("reservation"),
            )
            .unwrap();
    });
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .unwrap();
        let command = crate::infrastructure::test_support::read_framed_command(&stream).unwrap();
        let command: Value = serde_json::from_str(&command).unwrap();
        assert_eq!(command["action"], "wake");
        let mut stream = stream;
        writeln!(
            stream,
            "{}",
            json!({"type":"response","id":command["id"],
            "success":false,"error":"control queue is full"})
        )
        .unwrap();
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mutating_op_that_moves_the_cursor_sends_wake_hints() {
    let (_directory, context) = board();
    let recipient = rejecting_worker(&context);
    let sent = answered(
        &context,
        json!({"op": "send", "request": "approval", "recipient": "worker",
            "body": "Approved"}),
    )
    .await;
    recipient.join().unwrap();
    assert_eq!(sent["status"], "accepted", "{sent}");
    let warnings = sent["notification_warnings"].to_string();
    assert!(warnings.contains("wake hint failed for worker"), "{sent}");
    let worker = SwarmContext {
        member: "worker".into(),
        ..context.clone()
    };
    let inbox = direct(&worker, "inbox", json!([])).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 1, "the message is durable");
}

#[tokio::test]
async fn stop_by_op_settles_the_run() {
    let lifecycle = Arc::new(CountingLifecycle::default());
    let (_directory, context) = board_with(deadline(), lifecycle.clone());
    let receipt = answered(
        &context,
        json!({"op": "stop", "status": "blocked", "reason": "needs a decision"}),
    )
    .await;
    assert_eq!(receipt["outcome"], "blocked", "{receipt}");
    assert_eq!(lifecycle.settled.load(Ordering::SeqCst), 1, "settled once");
}

/// A read-only op reads the gate's `_status` and itself, never the event
/// cursor, a summary or the wake notifications, and never settles.
#[tokio::test]
async fn read_ops_do_not_touch_the_lifecycle() {
    let lifecycle = Arc::new(CountingLifecycle::default());
    let (_directory, context) = board_with(deadline(), lifecycle.clone());
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    for op in super::READ_ONLY_OPS {
        let arguments = match *op {
            "task" => json!({"op": "task", "task_id": 1}),
            other => json!({"op": other}),
        };
        answered(&context, arguments).await;
        let ops: Vec<String> = log.take().into_iter().map(|record| record.op).collect();
        assert_eq!(ops, ["_status", *op], "{op}");
    }
    assert_eq!(lifecycle.settled.load(Ordering::SeqCst), 0);
    // A mutation reads the cursor before and after, and notifies.
    answered(&context, json!({"op": "claim", "task_id": 1})).await;
    let ops: Vec<String> = log.take().into_iter().map(|record| record.op).collect();
    assert_eq!(
        ops[..4],
        ["_status", "_event_cursor", "claim", "_event_cursor"],
        "{ops:?}"
    );
    assert!(ops.contains(&"_notifications".to_owned()), "{ops:?}");
}

/// Owner decision (epic #2265, 2026-09-28): structured ops keep `op=run`'s
/// running gate, so a paused run answers no inbox, ack or read through them
/// (the epic's P4 is overruled).
#[tokio::test]
async fn inbox_and_ack_are_refused_while_paused() {
    let (_directory, context) = board();
    direct(&context, "send", json!(["m1", "coordinator", "hi"])).unwrap();
    crate::infrastructure::tools::call_work::off_the_runtime(|| context.pause("approval")).unwrap();
    for (op, arguments) in [
        ("inbox", r#"{"op":"inbox"}"#),
        ("ack", r#"{"op":"ack","message_id":1}"#),
        ("tasks", r#"{"op":"tasks"}"#),
    ] {
        let text = refused(&context, arguments).await;
        assert!(
            text.contains(&format!(
                "the swarm run is paused, so op={op} is unavailable"
            )),
            "{text}"
        );
    }
    let inbox = direct(&context, "inbox", json!([])).unwrap();
    assert_eq!(inbox[0]["status"], "accepted", "nothing was acknowledged");
}

#[tokio::test]
async fn an_op_before_create_points_the_founder_at_op_create() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let context = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    direct(
        &context,
        "_bootstrap",
        json!([1, "start", "/tmp/unused.sock"]),
    )
    .unwrap();
    let text = refused(&context, r#"{"op":"claim","task_id":1}"#).await;
    assert!(
        text.contains("status setup), so op=claim is unavailable"),
        "{text}"
    );
    assert!(text.contains(r#""op":"create""#), "{text}");
}

#[path = "swarm_board_ops_input_tests.rs"]
mod input;

#[path = "swarm_board_ops_notes_tests.rs"]
mod notes;
