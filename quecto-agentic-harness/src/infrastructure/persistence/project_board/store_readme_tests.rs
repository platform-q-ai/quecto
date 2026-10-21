//! #2660: the README every board write keeps.
use super::*;

/// #2660: the README a board's writes keep names its current layout:
/// every folder an artifact is kept in.
#[test]
fn the_readme_names_every_folder_the_board_keeps() {
    let readme = String::from_utf8_lossy(README);
    for folder in [
        "tasks/",
        "plans/",
        "runs/",
        "changes/",
        "reviews/",
        "discussions/",
    ] {
        assert!(readme.contains(folder), "{folder}: {readme}");
    }
}
