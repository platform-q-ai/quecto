//! The board server (epic #2705 B4) keeps its layers apart:
//!
//! - its transport ([`TRANSPORT`]: the listener, its limits and the
//!   client) knows no board: it names no layer of the crate's board, no
//!   store and no use case, and climbs out of its own module only to the
//!   crate's other HTTP adapters (never `super::super`);
//! - the listener reaches the board and the peer only through the handles
//!   it is given: it names [`INJECTED`], and answers a request only by
//!   calling the handler's `answer`, once;
//! - the server's front end ([`FRONT_END`]) builds no store, clock or file
//!   codec: composition does, so it names no persistence or time adapter.
use super::dependency_scan;

/// The board service's transport files.
const TRANSPORT: &[&str] = &[
    "src/infrastructure/http/board_service/mod.rs",
    "src/infrastructure/http/board_service/listener.rs",
    "src/infrastructure/http/board_service/limits.rs",
    "src/infrastructure/http/board_service/client.rs",
];

/// What the transport may never name: the board's layers, its store, its
/// use cases, and a path out of its module.
const TRANSPORT_NEVER_NAMES: &[&str] = &[
    "crate::application",
    "crate::domain",
    "crate::interface",
    "crate::composition",
    "crate::infrastructure::persistence",
    "ProjectBoardStore",
    "BoardOperations",
    "ServeBoard",
    "super::super",
];

/// The listener's injected handles.
const INJECTED: &[&str] = &["ServiceHandler", "PeerCredentials"];

/// The board server's front end.
const FRONT_END: &[&str] = &[
    "src/interface/cli/board_server/mod.rs",
    "src/interface/cli/board_server/serve.rs",
    "src/interface/cli/board_server/routes.rs",
];

/// What the front end may never name: the adapters composition builds.
const FRONT_END_NEVER_NAMES: &[&str] = &[
    "crate::infrastructure::persistence",
    "crate::infrastructure::time",
    "GitBoardStore",
    "MarkdownSnapshotFiles",
    "SystemBoardClock",
];

fn read(file: &str) -> String {
    assert!(
        dependency_scan::production_file(file),
        "{file} is a production module"
    );
    std::fs::read_to_string(file).unwrap_or_else(|error| panic!("{file}: {error}"))
}

#[test]
fn the_transport_knows_no_board() {
    for file in TRANSPORT {
        if let Err(hit) = dependency_scan::forbidden_hit(&read(file), TRANSPORT_NEVER_NAMES) {
            panic!("{file}: {hit}");
        }
    }
}

#[test]
fn the_listener_calls_injected_handles_only() {
    let source = read("src/infrastructure/http/board_service/listener.rs");
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
}

#[test]
fn the_front_end_builds_no_adapter() {
    for file in FRONT_END {
        if let Err(hit) = dependency_scan::forbidden_hit(&read(file), FRONT_END_NEVER_NAMES) {
            panic!("{file}: {hit}");
        }
    }
}

#[test]
fn the_scan_sees_what_it_refuses() {
    let named = "use crate::application::project_board::ports::ProjectBoardStore;";
    assert!(dependency_scan::forbidden_hit(named, TRANSPORT_NEVER_NAMES).is_err());
    let climbed = "fn f() { super::super::persistence::x(); }";
    assert!(dependency_scan::forbidden_hit(climbed, TRANSPORT_NEVER_NAMES).is_err());
    let commented = "/// [`ProjectBoardStore`]\nfn f() {}";
    assert!(dependency_scan::forbidden_hit(commented, TRANSPORT_NEVER_NAMES).is_ok());
}
