//! The board's front ends hand each request to the `BoardOperations` port
//! (epic #2705 B6), never to the use case behind it, so another adapter (a
//! board server's, B7) answers them exactly as the local one does:
//!
//! - `RunBoardOperation` is named, in production code, only where
//!   [`USE_CASE_NAMED_ONLY_IN`] allows: the board capability that declares
//!   it and composition, which wraps it in the local adapter;
//! - each front end's file ([`FRONT_ENDS`]) names the port, so the scan
//!   cannot pass by reading files that reach the board some other way.
use std::path::Path;

use super::dependency_scan;

/// The only production paths (a file, or a directory ending in `/`) that
/// may name the use case.
const USE_CASE_NAMED_ONLY_IN: &[&str] = &["src/application/project_board/", "src/composition/"];

/// The front ends' shared path to the board: the request runner, the site
/// that builds a board for a request, and the call that records it.
const FRONT_ENDS: &[&str] = &[
    "src/interface/tools/project_board/mod.rs",
    "src/interface/tools/project_board/site.rs",
    "src/interface/tools/project_board/recording.rs",
];

fn production_sources(dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            production_sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let file = path.display().to_string();
            if dependency_scan::production_file(&file) {
                files.push((file, std::fs::read_to_string(&path).expect("read")));
            }
        }
    }
}

fn allowed(file: &str, paths: &[&str]) -> bool {
    paths
        .iter()
        .any(|path| file == *path || (path.ends_with('/') && file.starts_with(path)))
}

#[test]
fn the_board_use_case_is_named_only_behind_the_port() {
    // The scan sees a use, a type and a call; a comment is not code.
    for naming in [
        "use crate::application::project_board::use_cases::RunBoardOperation;",
        "fn f(operations: &RunBoardOperation) {}",
        "fn f() { RunBoardOperation::new(parts); }",
    ] {
        assert!(dependency_scan::forbidden_hit(naming, &["RunBoardOperation"]).is_err());
    }
    let comment = "/// [`RunBoardOperation`]\nfn f() {}";
    assert!(dependency_scan::forbidden_hit(comment, &["RunBoardOperation"]).is_ok());
    let mut files = Vec::new();
    production_sources(Path::new("src"), &mut files);
    let scanned = |path: &str| files.iter().any(|(file, _)| file == path);
    assert!(
        FRONT_ENDS.iter().all(|front_end| scanned(front_end)),
        "the scan reads every front end"
    );
    let stray: Vec<&str> = files
        .iter()
        .filter(|(file, source)| {
            !allowed(file, USE_CASE_NAMED_ONLY_IN)
                && dependency_scan::forbidden_hit(source, &["RunBoardOperation"]).is_err()
        })
        .map(|(file, _)| file.as_str())
        .collect();
    assert!(
        stray.is_empty(),
        "RunBoardOperation named outside USE_CASE_NAMED_ONLY_IN: {stray:?}"
    );
}

#[test]
fn every_front_end_reaches_the_board_through_the_port() {
    let unported: Vec<&str> = FRONT_ENDS
        .iter()
        .copied()
        .filter(|file| {
            let source = std::fs::read_to_string(file).expect("read a front end");
            dependency_scan::forbidden_hit(&source, &["BoardOperations"]).is_ok()
        })
        .collect();
    assert!(
        unported.is_empty(),
        "front ends naming no BoardOperations: {unported:?}"
    );
}
