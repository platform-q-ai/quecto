//! How the board shows text from outside it (a name in a tree, a path
//! set aside): one allowlist, shared by every layer that shows such text
//! or checks it was shown so.
use regex::Regex;
use std::sync::LazyLock;

/// The characters a name from outside is shown with as they are: letters,
/// marks, numbers, punctuation and symbols. Anything else (spaces,
/// controls, format characters such as bidirectional overrides,
/// zero-width and tag characters, unassigned code points) is not.
static SHOWN_CHAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}]$").expect("a valid class"));

/// Whether `c` is shown as it is ([`SHOWN_CHAR`]).
pub fn shown_char(c: char) -> bool {
    SHOWN_CHAR.is_match(c.encode_utf8(&mut [0; 4]))
}

/// The folders the files of plans, runs, changes, reviews and
/// discussions live in, one per kind.
pub const ARTIFACT_FOLDERS: [&str; 5] = ["plans", "runs", "changes", "reviews", "discussions"];

/// Whether `path` is a set-aside file's path as a store shows it: one name
/// directly under `tasks/`, every character shown as it is, the one space
/// a store puts for a run of any other, or the `…` that marks a name it
/// cut.
pub fn shown_path(path: &str) -> bool {
    path.strip_prefix("tasks/")
        .is_some_and(|name| one_name(name) && shown_name(name))
}

/// Whether `path` is a path a store sets aside with no key (a stray) shows:
/// [`shown_path`], but directly under `tasks/` or any of
/// [`ARTIFACT_FOLDERS`], the folders a store reads.
pub fn shown_board_path(path: &str) -> bool {
    std::iter::once("tasks")
        .chain(ARTIFACT_FOLDERS)
        .filter_map(|folder| path.strip_prefix(folder)?.strip_prefix('/'))
        .any(|name| one_name(name) && shown_name(name))
}

/// [`shown_path`] for the file of a plan, run, change, review or
/// discussion: one name directly under `folder` (one of
/// [`ARTIFACT_FOLDERS`], its kind's), holding no `..`.
pub fn shown_artifact_path(folder: &str, path: &str) -> bool {
    ARTIFACT_FOLDERS.contains(&folder)
        && path
            .strip_prefix(folder)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|name| one_name(name) && !name.contains("..") && shown_name(name))
}

/// One path segment: not empty, no `/`, and neither `.` nor `..`.
fn one_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && name != "." && name != ".."
}

fn shown_name(name: &str) -> bool {
    name.chars().all(|c| shown_char(c) || c == ' ' || c == '…')
}

#[cfg(test)]
#[path = "shown_text_tests.rs"]
mod tests;
