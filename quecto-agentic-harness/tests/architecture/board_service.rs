//! The board server (epic #2705 B4) keeps its layers apart:
//!
//! - its transport ([`TRANSPORT`]: the listener and its limits) knows no
//!   board: it names no layer of the crate's board, no
//!   store and no use case, and climbs out of its own module only to the
//!   crate's other HTTP adapters (never `super::super`);
//! - the listener reaches the board and the peer only through the handles
//!   it is given: it names [`INJECTED`], and answers a request only by
//!   calling the handler's `answer`, once;
//! - the server's front end ([`FRONT_END`]) builds no store, clock or file
//!   adapter: composition does, so it names no persistence or time adapter.
//!
//! Each rule is checked twice: on the paths the file's parsed syntax names
//! (every `use` tree flattened, groups and relative paths resolved, by the
//! #1637 checker's [`dependency_paths_in`](super::dependency_paths_in)),
//! matched by whole-segment prefix and by name; and on its production
//! text, for a name used without a path.
use super::dependency_scan;

/// The board service's transport files.
const TRANSPORT: &[&str] = &[
    "src/infrastructure/http/board_service/mod.rs",
    "src/infrastructure/http/board_service/listener.rs",
    "src/infrastructure/http/board_service/limits.rs",
];

/// The crate paths the transport may never reach.
const TRANSPORT_NEVER_REACHES: &[&str] = &[
    "crate::application",
    "crate::domain",
    "crate::interface",
    "crate::composition",
    "crate::infrastructure::persistence",
];

/// What the transport may never name: the board's store and use cases,
/// and a path out of its module.
const TRANSPORT_NEVER_NAMES: &[&str] = &[
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

/// The crate paths the front end may never reach: the adapters
/// composition builds.
const FRONT_END_NEVER_REACHES: &[&str] = &[
    "crate::infrastructure::persistence",
    "crate::infrastructure::time",
];

/// What the front end may never name.
const FRONT_END_NEVER_NAMES: &[&str] = &["GitBoardStore", "SystemBoardClock"];

/// The listener's module, for resolving a synthetic source's paths.
const LISTENER: &str = "src/infrastructure/http/board_service/listener.rs";

fn read(file: &str) -> String {
    assert!(
        dependency_scan::production_file(file),
        "{file} is a production module"
    );
    std::fs::read_to_string(file).unwrap_or_else(|error| panic!("{file}: {error}"))
}

/// What `source` (of module `file`) names that the rule refuses: a path
/// under one of `reaches` or holding a segment of `names`, a path the
/// checker could not resolve, a source that does not parse, or a name of
/// `names` in its production text.
fn refused(file: &str, source: &str, reaches: &[&str], names: &[&str]) -> Vec<String> {
    let module = dependency_scan::module_path(file);
    let Some(paths) = super::dependency_paths_in(Some(module), source) else {
        return vec!["<does not parse>".into()];
    };
    let under = |path: &str, prefix: &str| {
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("::"))
    };
    let mut hits: Vec<String> = paths
        .into_iter()
        .filter(|path| {
            path == dependency_scan::UNRESOLVABLE
                || reaches.iter().any(|prefix| under(path, prefix))
                || path.split("::").any(|segment| names.contains(&segment))
        })
        .collect();
    if let Err(hit) = dependency_scan::forbidden_hit(source, names) {
        hits.push(hit);
    }
    hits
}

#[test]
fn the_transport_knows_no_board() {
    for file in TRANSPORT {
        let hits = refused(
            file,
            &read(file),
            TRANSPORT_NEVER_REACHES,
            TRANSPORT_NEVER_NAMES,
        );
        assert!(hits.is_empty(), "{file} names {hits:?}");
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
}

#[test]
fn the_front_end_builds_no_adapter() {
    for file in FRONT_END {
        let hits = refused(
            file,
            &read(file),
            FRONT_END_NEVER_REACHES,
            FRONT_END_NEVER_NAMES,
        );
        assert!(hits.is_empty(), "{file} names {hits:?}");
    }
}

#[test]
fn the_scan_sees_what_it_refuses() {
    let transport = |source: &str| {
        refused(
            LISTENER,
            source,
            TRANSPORT_NEVER_REACHES,
            TRANSPORT_NEVER_NAMES,
        )
    };
    for named in [
        "use crate::application::project_board::ports::ProjectBoardStore;",
        // Review round 1: a grouped import named no refused text whole.
        "use crate::{application::project_board::ports::Store, infrastructure::{persistence::x}};",
        "use crate::{domain as d};",
        "fn f() { super::super::persistence::x(); }",
        "fn f() { super::super::super::application::x(); }",
        "use crate::interface::*;",
        "fn f() -> crate::composition::Graph { loop {} }",
        "fn broken( {",
    ] {
        assert!(!transport(named).is_empty(), "{named}");
    }
    for allowed in [
        "/// [`ProjectBoardStore`]\nfn f() {}",
        "use super::limits::{self, Refusal};",
        "use crate::infrastructure::http::board_service::limits;",
        "use crate::{infrastructure::{http::board_service::limits}};",
        "fn f() { let _ = \"crate::application\"; }",
    ] {
        assert!(
            transport(allowed).is_empty(),
            "{allowed}: {:?}",
            transport(allowed)
        );
    }
    let front_end = refused(
        "src/interface/cli/board_server/serve.rs",
        "use crate::infrastructure::{http::x, time::{board_clock::Clock}};",
        FRONT_END_NEVER_REACHES,
        FRONT_END_NEVER_NAMES,
    );
    assert!(!front_end.is_empty(), "a grouped time adapter is refused");
}
