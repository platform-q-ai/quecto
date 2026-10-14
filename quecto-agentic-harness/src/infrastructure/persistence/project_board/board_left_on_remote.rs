//! What a partial clone fetches of the trees a read misses, and what it
//! knows a filtered fetch left on the remote (#2481 S10b): split out of
//! `board_tree` (review round 2).
use super::{Entry, FILTER_BYTES, Found, Git, Object, Unread, kind_of_mode, looked_up, objects};
use std::collections::{BTreeMap, BTreeSet};

/// What a partial clone knows a filtered fetch left on the remote: blobs
/// past every cap, never fetched, set aside as too large and carried by
/// their ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeftOnRemote {
    /// Trees this read fetched with the blob filter: a blob still missing
    /// under one was left there.
    pub trees: BTreeSet<String>,
    /// Blobs an earlier read found left there.
    pub blobs: BTreeSet<String>,
}

/// What a partial clone fetches of a tree it misses, or misses part of.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wanted {
    /// The tree alone, nothing under it: the root, the archive, or any
    /// other tree at the root, whose type is all a read judges.
    Alone(String),
    /// A live folder's tree and everything under it, but for any blob of
    /// [`FILTER_BYTES`] or more, which stays on the remote; or a file at
    /// the root by its id, which comes whole (git sends what it is asked
    /// for by id, whatever the filter).
    Filtered(String),
}

/// The real type and size of the objects of `entries`, the entries of the
/// live tree `tree`. A blob the filter left on the remote (`left`: under a
/// tree this read fetched filtered, or one found left before) is taken as
/// one past every cap ([`omit`]); anything else this clone misses asks for
/// `tree` to be fetched ([`Wanted::Filtered`]).
///
/// # Errors
/// What to fetch, or git fails.
pub(super) fn kinds_in(
    git: &Git,
    tree: &str,
    entries: &[Entry],
    left: &LeftOnRemote,
) -> Result<BTreeMap<String, Found>, Unread> {
    let mut kinds = objects(git, looked_up(entries))?;
    let filtered = left.trees.contains(tree);
    omit(entries, &mut kinds, |oid| {
        filtered || left.blobs.contains(oid)
    });
    let missing = entries
        .iter()
        .any(|entry| matches!(kinds.get(&entry.oid), Some(Found::Missing)));
    match missing {
        // A tree still missing after a fetch is asked for again, which the
        // store refuses to repeat.
        true => Err(Unread::Missing(vec![Wanted::Filtered(tree.to_string())])),
        false => Ok(kinds),
    }
}

/// `kinds` with each file of `entries` this clone misses that the filter
/// `left` on the remote taken for a blob past every cap, of at least
/// [`FILTER_BYTES`]: never fetched, set aside as too large, and carried by
/// its id.
fn omit(entries: &[Entry], kinds: &mut BTreeMap<String, Found>, left: impl Fn(&str) -> bool) {
    for entry in entries.iter().filter(|entry| left(&entry.oid)) {
        let found = kinds.get_mut(&entry.oid);
        if let (Some("blob"), Some(found @ Found::Missing)) = (kind_of_mode(&entry.mode), found) {
            *found = Found::Present(Object {
                kind: "blob".into(),
                size: FILTER_BYTES,
                omitted: true,
            });
        }
    }
}

/// The blobs of `kinds` a partial clone's filter left on the remote.
pub(super) fn omitted_in(kinds: &BTreeMap<String, Found>) -> impl Iterator<Item = String> + '_ {
    kinds.iter().filter_map(|(oid, found)| match found {
        Found::Present(object) if object.omitted => Some(oid.clone()),
        Found::Present(_) | Found::Missing => None,
    })
}
