//! The board a context or a hosted store reaches (#2278): composition's
//! handles, built once per board file, called in-process, and recorded in
//! the session's event log once the board has one.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::{SwarmBoard, SwarmBoardHandlesBuilder};
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::BoardOpLog;
use crate::composition::swarm::build_swarm_board_handles;
use crate::domain::swarm::BoardOpObservation;
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// The event log, in memory.
#[derive(Default)]
struct Recorded(Mutex<Vec<BoardOpObservation>>);

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }
}

impl Recorded {
    fn ops(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.op.clone())
            .collect()
    }

    fn text(&self) -> String {
        serde_json::to_string(&*self.0.lock().unwrap()).unwrap()
    }
}

fn context(checkout: &std::path::Path, member: &str, board: SwarmBoard) -> SwarmContext {
    SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: member.into(),
        board,
    }
}

fn deadline() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300
}

/// A run created by `parent` in `checkout`'s board.
fn create(parent: &SwarmContext) {
    std::fs::create_dir_all(parent.checkout.join(".quecto")).unwrap();
    parent
        .call(
            "create",
            json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline()]),
        )
        .unwrap();
}

static CONTEXT_BUILDS: AtomicUsize = AtomicUsize::new(0);

fn counted_for_context(
    location: BoardLocation,
    log: Option<Arc<dyn BoardOpLog>>,
) -> crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles {
    CONTEXT_BUILDS.fetch_add(1, Ordering::SeqCst);
    build_swarm_board_handles(location, log)
}

#[test]
fn a_context_calls_the_board_through_composed_handles_built_once_per_file() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let board = SwarmBoard::new(counted_for_context as SwarmBoardHandlesBuilder);
    let parent = context(first.path(), "parent", board.clone());
    create(&parent);
    assert_eq!(parent.summary().unwrap()["goal"], "ship");
    assert_eq!(
        CONTEXT_BUILDS.load(Ordering::SeqCst),
        1,
        "the composed handles, built once for the file"
    );
    let other = context(second.path(), "parent", board.clone());
    create(&other);
    assert_eq!(CONTEXT_BUILDS.load(Ordering::SeqCst), 2, "another file");
    assert_eq!(parent.summary().unwrap()["goal"], "ship");
    assert_eq!(
        CONTEXT_BUILDS.load(Ordering::SeqCst),
        3,
        "the board keeps the last file's handles"
    );
}

static HOSTED_BUILDS: AtomicUsize = AtomicUsize::new(0);

fn counted_for_host(
    location: BoardLocation,
    log: Option<Arc<dyn BoardOpLog>>,
) -> crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles {
    HOSTED_BUILDS.fetch_add(1, Ordering::SeqCst);
    build_swarm_board_handles(location, log)
}

#[test]
fn a_hosted_store_reads_through_composed_handles() {
    let checkout = tempfile::tempdir().unwrap();
    create(&context(
        checkout.path(),
        "parent",
        crate::composition::swarm::swarm_board(),
    ));
    let hosted = crate::infrastructure::tools::swarm_bridge::HostedStore::at(
        checkout.path().to_path_buf(),
        SwarmBoard::new(counted_for_host as SwarmBoardHandlesBuilder),
    );
    let run = hosted.hosted_run().unwrap().expect("a created run");
    assert_eq!(run.coordinator, "parent");
    assert_eq!(HOSTED_BUILDS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_board_with_a_log_records_one_swarm_op_per_call_and_no_argument_text() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    let recorded = Arc::new(Recorded::default());
    assert!(board.record_in(recorded.clone()));
    let parent = context(checkout.path(), "parent", board);
    create(&parent);
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    parent
        .call(
            "task_create",
            json!([
                "r1",
                format!("title {secret}"),
                [format!("acceptance {secret}")]
            ]),
        )
        .unwrap();
    let refused = parent.call("claim", json!([999])).unwrap_err();
    assert!(
        matches!(&refused, crate::domain::error::DomainError::Tool(text) if text == "swarm: \"unknown task\""),
        "{refused:?}"
    );
    parent.summary().unwrap();
    assert_eq!(
        recorded.ops(),
        ["create", "task_create", "claim", "summary"]
    );
    let text = recorded.text();
    assert!(!text.contains("sk-ant"), "no argument text: {text}");
    assert!(!text.contains("title"), "no argument text: {text}");
}

#[test]
fn a_board_records_in_its_first_log_only() {
    let checkout = tempfile::tempdir().unwrap();
    let board = crate::composition::swarm::swarm_board();
    let (first, second) = (Arc::new(Recorded::default()), Arc::new(Recorded::default()));
    assert!(board.record_in(first.clone()));
    assert!(!board.record_in(second.clone()), "the first log stays");
    create(&context(checkout.path(), "parent", board));
    assert_eq!(first.ops(), ["create"]);
    assert!(second.ops().is_empty());
}

static SESSION_LOG: std::sync::OnceLock<Arc<Recorded>> = std::sync::OnceLock::new();

/// A `board_op_log` that answers the in-memory log while the event log
/// is on.
fn session_log(enabled: bool, _log: &AuditLog) -> Option<Arc<dyn BoardOpLog>> {
    match enabled {
        true => Some(SESSION_LOG.get_or_init(Arc::default).clone()),
        false => None,
    }
}

#[test]
fn a_session_log_is_recorded_in_only_while_the_event_log_is_on() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:board").unwrap();
    let plain = SwarmBoard::new(build_swarm_board_handles);
    assert!(
        !plain.record_in_session(true, &log),
        "no board_op_log composed: nothing to record in"
    );
    let board = SwarmBoard::with_session_log(build_swarm_board_handles, session_log);
    assert!(
        !board.record_in_session(false, &log),
        "the event log is off"
    );
    assert!(board.record_in_session(true, &log));
    let checkout = tempfile::tempdir().unwrap();
    create(&context(checkout.path(), "parent", board));
    assert_eq!(SESSION_LOG.get().unwrap().ops(), ["create"]);
}

/// A board call blocks (a contended store waits up to its busy timeout),
/// so a wake accepted on an async worker is a harness bug: the
/// supervisor accepts wakes on the blocking pool only.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[cfg(debug_assertions)]
async fn accepting_a_wake_on_an_async_worker_is_refused_in_debug_builds() {
    let checkout = tempfile::tempdir().unwrap();
    let parent = context(
        checkout.path(),
        "parent",
        crate::composition::swarm::swarm_board(),
    );
    create(&parent);
    let on_worker = tokio::spawn(async move { parent.accept_wake(0) }).await;
    assert!(
        on_worker.is_err_and(|error| error.is_panic()),
        "a wake accepted on an async worker panics in a debug build"
    );
    let checkout = tempfile::tempdir().unwrap();
    let parent = context(
        checkout.path(),
        "parent",
        crate::composition::swarm::swarm_board(),
    );
    create(&parent);
    // On the blocking pool the call is made: whatever the board answers,
    // nothing panics.
    let _answer = crate::infrastructure::tools::call_work::spawn_blocking_in_call(move || {
        parent.accept_wake(0)
    })
    .await
    .unwrap();
}
