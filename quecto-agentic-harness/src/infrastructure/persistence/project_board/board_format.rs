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
///
/// **Grammar, binding on every later format.** The marker's first line,
/// up to its first LF (or its end), is `quecto-board <n>`: the ASCII text
/// `quecto-board`, one space, and `<n>`, a decimal number of ASCII digits
/// whose first is `1` to `9`, with nothing before, after or between. A
/// later format keeps this first line and raises `<n>`; what follows the
/// first line, other files in `format/` and the file's size are that
/// format's own. A reader takes any `<n>` above its own as a newer format,
/// however large (past what a `u64` holds included), and asks for an
/// upgrade; this quecto's marker is exactly `quecto-board 2` and an LF,
/// alone in `format/`, a regular file.
pub const FORMAT: &[u8] = b"quecto-board 2\n";
/// The format [`FORMAT`] names.
const FORMAT_VERSION: u64 = 2;

/// The name of the format marker's tree at a board's root.
pub const FORMAT_TREE: &str = "format";
/// The name of the marker's file in it.
pub const FORMAT_FILE: &str = "board";

/// What a board's format marker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Marker {
    /// This quecto's: [`FORMAT`], as it writes it.
    Current,
    /// A newer quecto's: its first line names a later format (its number,
    /// as written), whatever else the marker holds.
    Newer(String),
    /// Larger than this quecto reads to find the first line: a newer
    /// quecto's, or a hand edit's.
    Oversized(usize),
    /// Not one any quecto writes: why.
    Damaged(String),
}

/// The most of `format/board` read to find its first line: a newer
/// quecto's marker may be longer than this one's.
const MAX_MARKER_READ: usize = 64 * 1024;

/// The format number a marker's first line names, as written, when it
/// keeps the grammar of [`FORMAT`].
fn named_format(first: &[u8]) -> Option<&str> {
    let number = std::str::from_utf8(first)
        .ok()?
        .strip_prefix("quecto-board ")?;
    let bytes = number.as_bytes();
    let digits = bytes.first().is_some_and(|b| (b'1'..=b'9').contains(b))
        && bytes.iter().all(u8::is_ascii_digit);
    digits.then_some(number)
}

/// Whether the format number `number` (digits, no leading zero) is above
/// this quecto's, however large.
fn above(number: &str) -> bool {
    number
        .parse::<u64>()
        .map_or(true, |version| version > FORMAT_VERSION)
}

/// The format marker in `tree`, by [`FORMAT`]'s grammar.
///
/// # Errors
/// Objects this clone does not hold, or git fails.
pub(super) fn read_format(git: &Git, tree: &str, limits: Limits) -> Result<Marker, Unread> {
    let entries = listed(git, tree, limits.tree_bytes)?;
    let kinds = objects(git, looked_up(&entries))?;
    let missing = missing(&entries, &kinds);
    if !missing.is_empty() {
        return Err(Unread::Missing(missing));
    }
    let damaged = |why: String| Ok(Marker::Damaged(why));
    let Some(entry) = entries.iter().find(|entry| entry.name == FORMAT_FILE) else {
        return damaged("it holds no format/board".into());
    };
    let size = match kinds.get(&entry.oid) {
        Some(Found::Present(object)) if object.kind == "blob" => object.size,
        Some(Found::Present(object)) => {
            return damaged(format!("format/board is a {}, not a file", object.kind));
        }
        Some(Found::Missing) | None => return damaged("format/board names no object".into()),
    };
    if size > MAX_MARKER_READ {
        return Ok(Marker::Oversized(size));
    }
    let bytes = contents(git, &[(entry.oid.as_str(), size)])?
        .remove(&entry.oid)
        .unwrap_or_default();
    let first = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    match named_format(first) {
        Some(number) if above(number) => Ok(Marker::Newer(number.to_string())),
        Some(number) if number == FORMAT_VERSION.to_string() => {
            match (entries.len(), entry.mode.as_str(), bytes == FORMAT) {
                (1, REGULAR_FILE, true) => Ok(Marker::Current),
                (1, REGULAR_FILE, false) => {
                    damaged("format/board is not `quecto-board 2` alone".into())
                }
                (1, mode, _) => damaged(format!(
                    "format/board is of mode {mode}, not a regular file"
                )),
                (count, _, _) => damaged(format!(
                    "format holds {count} entries, not format/board alone"
                )),
            }
        }
        Some(older) => damaged(format!("it names format {older}, which no quecto writes")),
        None => damaged("format/board names no format".into()),
    }
}

/// The refusal of a board in a newer format.
pub(super) fn newer(version: &str) -> BoardStoreError {
    BoardStoreError::Unavailable(format!(
        "the board is in format {version}, written by a newer quecto; this quecto reads \
         format {FORMAT_VERSION}: upgrade quecto to read it"
    ))
}
