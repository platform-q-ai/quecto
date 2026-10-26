//! One artifact's file at a board commit, by its object (#2705 B9): what a
//! change pinned to that commit checks the artifact against. Only the
//! commit's root and the file's folder are listed, never a file read, so a
//! partial clone fetches those two trees alone ([`super::Wanted::Alone`]).
use super::{Entry, Git, Limits, REGULAR_FILE, TREE, Unread, listed, tree_of};

/// The object of the file `folder/name` at board commit `commit`, when
/// the commit's root lists `folder` once, as a tree, and that folder lists
/// `name` once, as a regular file; `None` otherwise (a name a read would
/// set aside, or none).
///
/// # Errors
/// [`Unread::Missing`] names the tree this clone lacks, to fetch alone
/// and look again; git fails, or a tree is past its bounds.
pub fn object_in(
    git: &Git,
    commit: &str,
    (folder, name): (&str, &str),
    limits: Limits,
) -> Result<Option<String>, Unread> {
    let root = listed(git, &tree_of(git, commit)?, limits.tree_bytes)?;
    let Some(tree) = only(&root, folder, TREE) else {
        return Ok(None);
    };
    // A tree this clone lacks is named to fetch alone: no file under it.
    let files = listed(git, &tree, limits.tree_bytes)?;
    Ok(only(&files, name, REGULAR_FILE))
}

/// The object `entries` lists under `name`, when it lists it once, of
/// `mode`.
fn only(entries: &[Entry], name: &str, mode: &str) -> Option<String> {
    let named: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.raw == name.as_bytes())
        .collect();
    match named.as_slice() {
        [entry] if entry.mode == mode => Some(entry.oid.clone()),
        _ => None,
    }
}
