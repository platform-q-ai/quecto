//! The board's format marker (`board_tree`, #2527): a `format` tree at the
//! root of one regular file, `board`, reading [`FORMAT`]. A quecto before
//! the task files became markdown takes any root tree but `tasks` for a
//! clash, so it never reads, and never writes over, a board whose tasks it
//! would not see.
use super::Git;
use super::{Found, contents, objects};
use super::{Limits, REGULAR_FILE, Unread, listed, looked_up, missing};
use crate::application::project_board::ports::BoardStoreError;

/// The board's format marker: the file `format/board` of a board's root.
pub const FORMAT: &[u8] = b"quecto-board 2\n";
/// The format [`FORMAT`] names.
const FORMAT_VERSION: u64 = 2;
/// The largest format marker read.
const MAX_FORMAT_BYTES: usize = 64;

/// The name of the format marker's tree at a board's root.
pub const FORMAT_TREE: &str = "format";
/// The name of the marker's file in it.
pub const FORMAT_FILE: &str = "board";

/// The format marker in `tree` is this quecto's: one regular file,
/// [`FORMAT_FILE`], reading [`FORMAT`]. A newer format asks for an
/// upgrade; anything else is not a board quecto writes.
pub(super) fn read_format(git: &Git, tree: &str, limits: Limits) -> Result<(), Unread> {
    let entries = listed(git, tree, limits.tree_bytes)?;
    let kinds = objects(git, looked_up(&entries))?;
    let missing = missing(&entries, &kinds);
    if !missing.is_empty() {
        return Err(Unread::Missing(missing));
    }
    let clash = || {
        Unread::Failed(BoardStoreError::Clash(
            "its format marker (format/board) is not one quecto writes, so it is not a board"
                .into(),
        ))
    };
    let [entry] = entries.as_slice() else {
        return Err(clash());
    };
    let size = match kinds.get(&entry.oid) {
        Some(Found::Present(object)) if object.kind == "blob" => object.size,
        Some(Found::Present(_) | Found::Missing) | None => return Err(clash()),
    };
    match (
        entry.name.as_str(),
        entry.mode.as_str(),
        size <= MAX_FORMAT_BYTES,
    ) {
        (FORMAT_FILE, REGULAR_FILE, true) => {}
        _ => return Err(clash()),
    }
    let bytes = contents(git, &[(entry.oid.as_str(), size)])?
        .remove(&entry.oid)
        .unwrap_or_default();
    let version = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| text.strip_prefix("quecto-board "))
        .and_then(|text| text.strip_suffix('\n'))
        .filter(|number| !number.starts_with('0') && number.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|number| number.parse::<u64>().ok());
    match version {
        Some(FORMAT_VERSION) => {
            debug_assert_eq!(bytes, FORMAT, "the marker this quecto writes");
            Ok(())
        }
        Some(newer) if newer > FORMAT_VERSION => {
            Err(Unread::Failed(BoardStoreError::Unavailable(format!(
                "the board is in format {newer}, written by a newer quecto; this quecto reads \
                 format {FORMAT_VERSION}: upgrade quecto to read it"
            ))))
        }
        Some(_) | None => Err(clash()),
    }
}
