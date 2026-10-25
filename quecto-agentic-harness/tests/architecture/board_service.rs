//! The board server (epic #2705 B4) keeps its layers apart, each rule an
//! allowlist (AGENTS.md) of the crate paths a file may reach:
//!
//! - its transport ([`TRANSPORT`]: the listener and its limits) reaches
//!   only its own module, [`TRANSPORT_REACHES`], and external crates: it
//!   knows no board;
//! - the listener reaches the board and the peer only through the handles
//!   it is given: it names [`INJECTED`], and answers a request only by
//!   calling the handler's `answer`, once;
//! - the server's front end ([`FRONT_END`]) reaches only
//!   [`FRONT_END_REACHES`]: the board's ports, use cases and values, the
//!   CLI, and the listener's surface. Composition builds every adapter.
//!
//! Paths are read from the parsed syntax by the #1637 checker's
//! [`dependency_paths_in`](super::dependency_paths_in): every `use` tree
//! flattened, groups and relative paths resolved, a module alias (which
//! no rule could follow) unresolvable and so refused.
use super::dependency_scan;

/// The board service's transport files.
const TRANSPORT: &[&str] = &[
    "src/infrastructure/http/board_service/mod.rs",
    "src/infrastructure/http/board_service/listener.rs",
    "src/infrastructure/http/board_service/limits.rs",
    "src/infrastructure/http/board_service/client.rs",
    "src/infrastructure/http/board_service/events.rs",
    "src/infrastructure/http/board_service/sources.rs",
    "src/infrastructure/http/board_service/replies.rs",
    "src/infrastructure/http/board_service/socket.rs",
];

/// The crate paths the transport may reach.
const TRANSPORT_REACHES: &[&str] = &["crate::infrastructure::http::board_service"];

/// The listener's injected handles.
const INJECTED: &[&str] = &["ServiceHandler", "PeerCredentials"];

/// The board server's front end.
const FRONT_END: &[&str] = &[
    "src/interface/cli/board_server/mod.rs",
    "src/interface/cli/board_server/serve.rs",
    "src/interface/cli/board_server/routes.rs",
    "src/interface/cli/board_server/token.rs",
    "src/interface/cli/board_server/op.rs",
    "src/interface/cli/board_server/feed.rs",
];

/// The crate paths the front end may reach: each a path, or the module
/// whose items it may name.
const FRONT_END_REACHES: &[&str] = &[
    "crate::application::project_board::ports",
    "crate::application::project_board::use_cases",
    "crate::application::project_board::dto",
    "crate::application::project_board::telemetry::OpFacts",
    // B8: what a client sends beside an end (its facts) and a start (its undo).
    "crate::application::project_board::run_facts::RunFacts",
    "crate::application::project_board::start::StartUndo",
    "crate::domain::project_board::value_objects::vocabulary::TaskStatus",
    "crate::domain::project_board::entities::claim::Identity",
    "crate::domain::project_board::value_objects::telemetry",
    "crate::domain::shared::redaction",
    "crate::interface::cli",
    // B7: an op is read and answered by the front ends' own path.
    "crate::interface::tools::project_board",
    "crate::infrastructure::http::board_service::listener::ServiceHandler",
    "crate::infrastructure::http::board_service::listener::ServiceMethod",
    "crate::infrastructure::http::board_service::listener::ServiceReply",
    "crate::infrastructure::http::board_service::listener::ServiceRequest",
    "crate::infrastructure::http::board_service::listener::PeerCredentials",
    "crate::infrastructure::http::board_service::listener::ServiceParts",
    "crate::infrastructure::http::board_service::listener::ConnectionLimits",
    "crate::infrastructure::http::board_service::listener::bind",
    "crate::infrastructure::http::board_service::listener::serve",
    "crate::infrastructure::http::board_service::listener::Transport",
    // B10: a streamed reply.
    "crate::infrastructure::http::board_service::listener::ServiceStream",
    "crate::infrastructure::http::board_service::listener::StreamStep",
    "crate::infrastructure::http::board_service::listener::StreamWake",
    "crate::infrastructure::http::board_service::listener::Listener",
    "crate::infrastructure::http::board_service::listener::bind_tcp",
    "crate::infrastructure::http::board_service::client::BoardServiceClient",
    "crate::infrastructure::http::board_service::client::READ_REPLY",
];

/// The listener's module, for resolving a synthetic source's paths.
const LISTENER: &str = "src/infrastructure/http/board_service/listener.rs";

/// The front end's root, likewise.
const FRONT_END_ROOT: &str = "src/interface/cli/board_server/mod.rs";

fn read(file: &str) -> String {
    assert!(
        dependency_scan::production_file(file),
        "{file} is a production module"
    );
    std::fs::read_to_string(file).unwrap_or_else(|error| panic!("{file}: {error}"))
}

/// The crate paths `source` (of module `file`) reaches that `reaches`
/// does not allow (each allowed path is matched whole, or as the parent of
/// a longer one), an unresolvable path, or `<does not parse>`. Paths into
/// external crates are free.
fn outside(file: &str, source: &str, reaches: &[&str]) -> Vec<String> {
    let module = dependency_scan::module_path(file);
    let Some(paths) = super::dependency_paths_in(Some(module), source) else {
        return vec!["<does not parse>".into()];
    };
    let within = |path: &str, allowed: &str| {
        path == allowed
            || path
                .strip_prefix(allowed)
                .is_some_and(|rest| rest.starts_with("::"))
    };
    paths
        .into_iter()
        .filter(|path| path.starts_with("crate::"))
        .filter(|path| !reaches.iter().any(|allowed| within(path, allowed)))
        .collect()
}

#[test]
fn the_transport_reaches_only_its_own_module() {
    for file in TRANSPORT {
        let hits = outside(file, &read(file), TRANSPORT_REACHES);
        assert!(hits.is_empty(), "{file} reaches {hits:?}");
    }
}

#[test]
fn the_listener_calls_injected_handles_only() {
    let source = read(LISTENER);
    for handle in INJECTED {
        assert!(
            dependency_scan::forbidden_hit(&source, &[handle]).is_err(),
            "the listener names {handle}"
        );
    }
    let text = dependency_scan::production_text(&source).expect("the listener parses");
    assert_eq!(
        text.matches(".answer(").count(),
        1,
        "the listener answers through its handler's `answer`, once"
    );
    assert_eq!(
        text.matches("handler.stream(").count(),
        1,
        "a streamed route is opened through its handler's `stream`, once (B10)"
    );
}

#[test]
fn the_front_end_reaches_only_its_allowlist() {
    for file in FRONT_END {
        let hits = outside(file, &read(file), FRONT_END_REACHES);
        assert!(hits.is_empty(), "{file} reaches {hits:?}");
    }
}

#[test]
fn the_scan_sees_what_it_refuses() {
    let transport = |source: &str| outside(LISTENER, source, TRANSPORT_REACHES);
    for refused in [
        // Review round 4: an adapter the listener must never build.
        "use crate::infrastructure::time::board_clock::SystemBoardClock;",
        "use crate::application::project_board::ports::ProjectBoardStore;",
        // Review round 1: a grouped import named no refused text whole.
        "use crate::{application::project_board::ports::Store, infrastructure::{persistence::x}};",
        "use crate::{domain as d};",
        "fn f() { super::super::web_fetch::x(); }",
        "fn f() { super::super::super::persistence::x(); }",
        "use crate::interface::*;",
        "fn f() -> crate::composition::Graph { loop {} }",
        "fn broken( {",
    ] {
        assert!(!transport(refused).is_empty(), "{refused}");
    }
    for allowed in [
        "/// [`ProjectBoardStore`]\nfn f() {}",
        "use super::limits::{self, Refusal};",
        "use crate::infrastructure::http::board_service::limits;",
        "use crate::{infrastructure::{http::board_service::limits}};",
        "use hyper::body::Bytes;",
        "fn f() { let _ = \"crate::application\"; }",
    ] {
        let hits = transport(allowed);
        assert!(hits.is_empty(), "{allowed}: {hits:?}");
    }
    let front_end = |source: &str| outside(FRONT_END_ROOT, source, FRONT_END_REACHES);
    for refused in [
        // Review round 4: the kernel's peer reader is composition's to pick.
        "use crate::infrastructure::http::board_service::listener::SocketPeerCredentials;",
        "use crate::infrastructure::{http::x, time::{board_clock::Clock}};",
        "use crate::infrastructure::http::board_service::listener;",
        "use crate::infrastructure::persistence::project_board::store::GitBoardStore;",
        "use crate::application::project_board::operations::ListSummaries;",
    ] {
        assert!(!front_end(refused).is_empty(), "{refused}");
    }
    for allowed in [
        "use crate::infrastructure::http::board_service::listener::{ServiceReply, bind};",
        "use crate::application::project_board::ports::ServeError;",
        "use super::BoardServerHandles;",
    ] {
        let hits = front_end(allowed);
        assert!(hits.is_empty(), "{allowed}: {hits:?}");
    }
}

/// The remote client (B7): its adapter, its transport and the composition
/// that picks it.
const REMOTE: &[&str] = &[
    "src/infrastructure/http/board_remote.rs",
    "src/infrastructure/http/board_watch.rs",
    "src/infrastructure/http/board_service/client.rs",
    "src/infrastructure/http/board_service/events.rs",
    "src/composition/bootstrap/project_board_remote.rs",
    // B8: a start begun there and launched here, an end sent with its facts.
    "src/infrastructure/http/board_remote_runs.rs",
    "src/composition/bootstrap/project_board_remote_runs.rs",
];

/// B7 acceptance: a remote client never reads the clock (the server's is
/// the one clock), and never decodes the board (a write is one request,
/// whose answer comes back rendered).
#[test]
fn a_remote_client_reads_no_clock_and_decodes_no_board() {
    let clock = [
        "BoardClock",
        "board_clock",
        "remote_time",
        "SystemTime",
        "Instant",
        "Timestamp",
        "humantime",
        "chrono",
        "TimeSync",
        "ClockSync",
        "KernelTimeSync",
        "kernel_time_sync",
        "adjtimex",
        "ntp_adjtime",
    ];
    let board = [
        "BoardSnapshot",
        "ProjectBoardStore",
        "QueryBoard",
        "BoardListings",
        "artifacts",
        "RunBoardOperation",
        ".read()",
    ];
    for file in REMOTE {
        let source = read(file);
        let unclocked = unclocked(&source);
        assert_eq!(
            dependency_scan::forbidden_hit(&unclocked, &clock),
            Ok(()),
            "{file}"
        );
        assert_eq!(
            dependency_scan::forbidden_hit(&source, &board),
            Ok(()),
            "{file}"
        );
    }
    let seen = "fn f() { let _ = std::time::SystemTime::now(); }";
    assert!(dependency_scan::forbidden_hit(seen, &clock).is_err());
    // The stripping takes the refusal's name alone: any other clock name
    // in the same source is still caught.
    for still in [
        "use crate::application::project_board::ports::{BoardClockError, BoardClock};",
        "fn f() -> BoardClockError { BoardClockError::OutOfRange(String::new()) }",
        "fn f(_: &dyn BoardClock) {}",
        "fn f() { let _ = (BoardClockError::Unsynced(String::new()), KernelTimeSync); }",
    ] {
        let stripped = unclocked(still);
        assert!(
            dependency_scan::forbidden_hit(&stripped, &clock).is_err(),
            "{still}"
        );
    }
}

/// `source` without the one clock name a remote client may hold: the
/// clock's error type, to hand back the server's `clock_unsynced` refusal
/// (#2705). Naming it reads no clock.
fn unclocked(source: &str) -> String {
    source
        .replace("BoardClockError::Unsynced", "")
        .replace("BoardClockError,", "")
}
