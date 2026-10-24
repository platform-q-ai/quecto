//! The board's format marker (`board_tree`, #2527): a `format` tree at the
//! root of one regular file, `board`, reading [`FORMAT`]. A quecto before
//! the task files became markdown takes any root tree but `tasks` for a
//! clash, so it never reads, and never writes over, a board whose tasks it
//! would not see. Format 3 (#2481 S9) adds the other kinds' folders and the
//! archive to the root: a quecto of format 2 asks for an upgrade, never
//! takes them for a clash, and this one reads a board of format 2 as it is.
use super::{Found, contents};
use super::{LeftOnRemote, Limits, REGULAR_FILE, Unread, kinds_in, listed};
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
/// upgrade; this quecto's marker is exactly `quecto-board 3` and an LF,
/// alone in `format/`, a regular file.
pub const FORMAT: &[u8] = b"quecto-board 3\n";
/// The format [`FORMAT`] names.
const FORMAT_VERSION: u64 = 3;
/// The formats this quecto reads, each by its number and its marker's
/// bytes: format 2 (tasks alone) as it is, and its own.
const READ_FORMATS: [(&str, &[u8]); 2] = [("2", b"quecto-board 2\n"), ("3", FORMAT)];

/// The fence's marker (epic #2705 B13): format 4, a board moved to a
/// board server. Its `format` tree holds exactly this marker, a regular
/// file, and [`MOVED_FILE`], a regular file naming where the board moved
/// (`BoardMoved`'s grammar); this quecto neither reads nor writes such a
/// board, and answers where it moved.
pub const MOVED_FORMAT: &[u8] = b"quecto-board 4\n";
/// The name of the fence's file in `format/` that says where the board
/// moved.
pub const MOVED_FILE: &str = "MOVED";

/// The name of the format marker's tree at a board's root.
pub const FORMAT_TREE: &str = "format";
/// The name of the marker's file in it.
pub const FORMAT_FILE: &str = "board";

/// What a board's format marker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Marker {
    /// One this quecto reads: [`FORMAT`], as it writes it, or the format
    /// before it, read as it is.
    Current,
    /// A newer quecto's: its first line names a later format (its number,
    /// as written), whatever else the marker holds.
    Newer(String),
    /// Larger than this quecto reads to find the first line: a newer
    /// quecto's, or a hand edit's. Its size as told: `N bytes`, or `at
    /// least N bytes` for one a partial clone left on the remote.
    Oversized(String),
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
pub(super) fn read_format(
    git: &impl super::Reader,
    tree: &str,
    limits: Limits,
    left: &LeftOnRemote,
) -> Result<Marker, Unread> {
    let entries = listed(git, tree, limits.tree_bytes)?;
    let kinds = kinds_in(git, tree, &entries, left)?;
    let damaged = |why: String| Ok(Marker::Damaged(why));
    let Some(entry) = entries.iter().find(|entry| entry.name == FORMAT_FILE) else {
        return damaged("it holds no format/board".into());
    };
    let (size, omitted) = match kinds.get(&entry.oid) {
        Some(Found::Present(object)) if object.kind == "blob" => (object.size, object.omitted),
        Some(Found::Present(object)) => {
            return damaged(format!("format/board is a {}, not a file", object.kind));
        }
        Some(Found::Missing) | None => return damaged("format/board names no object".into()),
    };
    match (size > MAX_MARKER_READ, omitted) {
        (true, false) => return Ok(Marker::Oversized(format!("{size} bytes"))),
        (true, true) => return Ok(Marker::Oversized(format!("at least {size} bytes"))),
        // A blob left on the remote is past every cap.
        (false, omitted) => assert!(!omitted, "an omitted marker is past what is read"),
    }
    let bytes = contents(git, &[(entry.oid.as_str(), size)])?
        .remove(&entry.oid)
        .unwrap_or_default();
    let first = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let read = |number: &str| READ_FORMATS.iter().find(|(read, _)| *read == number);
    match named_format(first) {
        Some(number) if above(number) => Ok(Marker::Newer(number.to_string())),
        Some(number) => match read(number) {
            Some((_, marker)) => match (entries.len(), entry.mode.as_str(), bytes == *marker) {
                (1, REGULAR_FILE, true) => Ok(Marker::Current),
                (1, REGULAR_FILE, false) => {
                    damaged(format!("format/board is not `quecto-board {number}` alone"))
                }
                (1, mode, _) => damaged(format!(
                    "format/board is of mode {mode}, not a regular file"
                )),
                (count, _, _) => damaged(format!(
                    "format holds {count} entries, not format/board alone"
                )),
            },
            None => damaged(format!("it names format {number}, which no quecto writes")),
        },
        None => damaged("format/board names no format".into()),
    }
}

/// The refusal of a board in a newer format.
pub(super) fn newer(version: &str) -> BoardStoreError {
    BoardStoreError::Unavailable(format!(
        "the board is in format {version}, written by a newer quecto; this quecto reads \
         formats {}: upgrade quecto to read it",
        READ_FORMATS.map(|(read, _)| read).join(" and ")
    ))
}
