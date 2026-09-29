//! The coordination board's graph is composed in one place (#2278, epic
//! #2265): `src/composition/swarm.rs` binds the SQLite repository, the
//! random ids and the use cases, and `SwarmContext`/`HostedStore` reach
//! the board only through the handles it builds. No production file of
//! the tool adapters or the interface names the board's persistence, its
//! ids or its repository, nor the retired Python board interpreter
//! (`swarm_board_worker`); outside the board's own persistence module,
//! only composition constructs the repository or the ids.
use std::path::Path;

use super::dependency_scan;

/// The one production file that composes the board.
const COMPOSITION: &str = "src/composition/swarm.rs";

/// The board's own persistence module, which defines what composition
/// binds.
const PERSISTENCE: &str = "src/infrastructure/persistence/swarm_board/";

/// The board's callers: the tool adapters and the interface.
const CALLERS: &[&str] = &["src/infrastructure/tools", "src/interface"];

/// What a caller never names: the board's graph, and the retired Python
/// board interpreter the callers used before #2278.
const GRAPH: &[&str] = &[
    "persistence::swarm_board",
    "Uuid4Ids",
    "SqliteBoardRepository",
    "build_swarm_board_handles_with",
    "swarm_board_worker",
];

/// What only composition constructs outside the persistence module.
const CONSTRUCTED: &[&str] = &["Uuid4Ids", "SqliteBoardRepository"];

/// Every production file under `dir`, with its text.
fn production_files(dir: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    super::collect_rs_files(Path::new(dir), &mut files);
    files
        .into_iter()
        .filter_map(|entry| {
            let (file, source) = entry.split_once(":\n").expect("a collected file");
            match dependency_scan::production_file(file) {
                true => Some((file.to_owned(), source.to_owned())),
                false => None,
            }
        })
        .collect()
}

#[test]
fn swarm_board_graph_is_built_only_in_composition() {
    let mut offences = Vec::new();
    for dir in CALLERS {
        for (file, source) in production_files(dir) {
            if let Some(offence) = super::file_offence(&file, &source, GRAPH) {
                offences.push(format!("{file}: {offence}"));
            }
        }
    }
    for (file, source) in production_files("src") {
        let owner = file == COMPOSITION || file.starts_with(PERSISTENCE);
        if owner {
            continue;
        }
        if let Some(offence) = super::file_offence(&file, &source, CONSTRUCTED) {
            offences.push(format!("{file}: {offence}"));
        }
    }
    assert!(
        offences.is_empty(),
        "the board's graph is composed only in {COMPOSITION}:\n{}",
        offences.join("\n")
    );
    // The rule is not vacuous: composition does bind the graph.
    let composition = std::fs::read_to_string(COMPOSITION).expect("read the board's composition");
    for name in CONSTRUCTED {
        assert!(
            super::file_offence(COMPOSITION, &composition, &[name]).is_some(),
            "{COMPOSITION} constructs {name}"
        );
    }
}
