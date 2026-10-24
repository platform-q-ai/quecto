//! The fence (epic #2705 B13): the one commit on `quecto/board` that hands
//! the board to a board server. Its root is the board's root as it stood,
//! but its `format` tree holds the format-4 marker ([`MOVED_FORMAT`]) and
//! `MOVED` ([`BoardMoved::file`]), and strays are dropped as by every
//! write. Every quecto that reads format 3 refuses format 4 for reads and
//! writes alike; this one answers that the board moved.
use super::{FORMAT_FILE, FORMAT_TREE, MOVED_FILE, MOVED_FORMAT};
use super::{Git, Limits, TREE, Unread, kind_of_mode, listed, root_tree, tree_of};
use crate::application::project_board::ports::BoardStoreError;
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use std::collections::BTreeMap;

/// The README's name at a board's root.
const README: &str = "README.md";

/// The files of a fence's `format` tree, by name: its marker and `MOVED`.
pub fn fence_files(moved: &BoardMoved) -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        (FORMAT_FILE, MOVED_FORMAT.to_vec()),
        (MOVED_FILE, moved.file()),
    ])
}

/// The root of a fence: `root` (a board's root entries, by name, as
/// `mktree` takes them: `<mode> <type> <oid>`) with its `format` tree
/// replaced by `format`, a tree's id. Only the README, `tasks`, the other
/// folders and the archive are carried, as they are: anything else is a
/// stray, which every write drops.
pub fn fenced_root(root: &BTreeMap<String, String>, format: &str) -> BTreeMap<String, String> {
    let carried = |name: &str| name == README || root_tree(name);
    let mut fenced: BTreeMap<String, String> = root
        .iter()
        .filter(|(name, _)| carried(name))
        .map(|(name, entry)| (name.clone(), entry.clone()))
        .collect();
    let replaced = fenced.insert(FORMAT_TREE.to_string(), format!("{TREE} tree {format}"));
    assert!(replaced.is_none(), "the format tree is never carried");
    fenced
}

/// The root entries of board commit `commit`, by name, as `mktree` takes
/// them; a name that is not UTF-8 (a stray) is left out.
///
/// # Errors
/// git fails, the root is past its bound, or this clone does not hold it.
pub fn root_entries(
    git: &Git,
    commit: &str,
    limits: Limits,
) -> Result<BTreeMap<String, String>, BoardStoreError> {
    let failed = |unread: Unread| match unread {
        Unread::Failed(error) => error,
        Unread::Missing(_) => {
            BoardStoreError::Unavailable(format!("this clone does not hold the root of {commit}"))
        }
    };
    let tree = tree_of(git, commit).map_err(failed)?;
    let entries = listed(git, &tree, limits.tree_bytes).map_err(failed)?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let name = String::from_utf8(entry.raw).ok()?;
            let kind = kind_of_mode(&entry.mode)?;
            Some((name, format!("{} {kind} {}", entry.mode, entry.oid)))
        })
        .collect())
}

#[cfg(test)]
#[path = "board_fence_tests.rs"]
mod tests;
