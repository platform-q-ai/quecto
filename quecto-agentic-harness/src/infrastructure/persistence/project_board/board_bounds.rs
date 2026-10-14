//! The bounds a change to the board keeps (`board_tree`): the entries of
//! its `tasks/` tree a write leaves, which a clear never grows; each other
//! folder's entries and file bytes as the write leaves it (#2481 S10a),
//! the bounds a read takes of it; and the entries and file bytes a read
//! reads in all.
use super::folders::FOLDERS;
use super::writes::FolderAfter;
use super::{Board, Limits, MAX_BOARD_ENTRIES, MAX_TASK_ENTRIES, Unread, task_file, too_large};
use crate::application::project_board::ports::BoardStoreError;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};

/// What a clear keeps of the names listed twice it resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearKeeps {
    /// No copy: it only takes files away.
    Nothing,
    /// A copy of each of these paths, at least one.
    Copies(Vec<String>),
}

/// The most kept paths a refusal names before it says how many more.
const SHOWN_PATHS: usize = 8;

/// A board of `entries` in all its folders, and `bytes` of files to read,
/// keeps within what a read takes: [`MAX_BOARD_ENTRIES`] and
/// `limits.board_bytes`. A read checks it before it reads any file's bytes.
///
/// # Errors
/// Past either bound.
pub(super) fn within_read(entries: usize, bytes: usize, limits: Limits) -> Result<(), Unread> {
    match (entries <= MAX_BOARD_ENTRIES, bytes <= limits.board_bytes) {
        (true, true) => Ok(()),
        (false, _) => Err(too_large(format!(
            "the board's trees hold {entries} entries; the most read is {MAX_BOARD_ENTRIES}"
        ))),
        (true, false) => Err(too_large(format!(
            "the board's files hold {bytes} bytes; the most read is {}",
            limits.board_bytes
        ))),
    }
}

impl Board {
    /// The board, with `written` in place and its other folders as
    /// `folders`, keeps within what a write leaves: the entries of its
    /// `tasks/` tree (each task one file, each file set aside one), at most
    /// [`MAX_TASK_ENTRIES`], and the task-file bytes a read reads; each
    /// other folder within the entries and file bytes a read takes of it;
    /// and the board within its own. Every write checks it before it builds
    /// anything.
    ///
    /// # Errors
    /// An invalid write past any bound.
    pub fn within_bounds(
        &self,
        written: &[(&Task, Vec<u8>)],
        folders: &BTreeMap<&'static str, FolderAfter<'_>>,
        limits: Limits,
    ) -> Result<(), BoardStoreError> {
        folders_within(folders)?;
        self.entries_within(written, MAX_TASK_ENTRIES, folders)?;
        self.bytes_within(written, limits, folders)
    }

    /// The board a clear of `before` leaves keeps within what a read takes:
    /// a clear never adds an entry, so a board read past what a write
    /// leaves (one written before the task files became markdown) can
    /// still be cleared, file by file, back within it. A copy a clear keeps
    /// of a name listed twice is read again, so its bytes count: keeping
    /// one may take a folder, `tasks/` or the board past what a read takes
    /// (S10b), and such a clear is invalid, naming the copies it keeps and
    /// the ways out. A clear never grows an entry count, kept copies or
    /// not: each takes the place of two or more.
    ///
    /// # Errors
    /// An invalid clear past any bound.
    pub fn within_clear_bounds(
        &self,
        before: &Board,
        folders: &BTreeMap<&'static str, FolderAfter<'_>>,
        limits: Limits,
        keeps: &ClearKeeps,
    ) -> Result<(), BoardStoreError> {
        let cap = MAX_TASK_ENTRIES.max(before.entries(&[]));
        let told = |error: BoardStoreError| match (keeps, error) {
            (ClearKeeps::Copies(paths), BoardStoreError::InvalidWrite(why)) => {
                let shown: Vec<&str> = paths.iter().take(SHOWN_PATHS).map(String::as_str).collect();
                let more = match paths.len().saturating_sub(SHOWN_PATHS) {
                    0 => String::new(),
                    more => format!(" and {more} more"),
                };
                let copies = match paths.len() {
                    1 => "copy",
                    _ => "copies",
                };
                BoardStoreError::InvalidWrite(format!(
                    "{why}, once this clear keeps its {copies} of {}{more}: keep a smaller copy, \
                     or drop every copy instead",
                    shown.join(", ")
                ))
            }
            (ClearKeeps::Copies(_) | ClearKeeps::Nothing, other) => other,
        };
        folders_within(folders).map_err(told)?;
        self.entries_within(&[], cap, folders)?;
        self.bytes_within(&[], limits, folders).map_err(told)
    }

    /// The entries of the `tasks/` tree with `written` in place. A file a
    /// clear kept that a read will read as a task is in `kept` until then;
    /// its id is not among the tasks, so it is counted once.
    fn entries(&self, written: &[(&Task, Vec<u8>)]) -> usize {
        let ids = written.iter().map(|(task, _)| &task.fields().id);
        let after: BTreeSet<&Slug> = self.tasks.keys().chain(ids).collect();
        let copies: usize = self.repeated.iter().map(|(_, copies)| copies.len()).sum();
        after.len() + self.kept.len() + copies
    }

    fn entries_within(
        &self,
        written: &[(&Task, Vec<u8>)],
        cap: usize,
        folders: &BTreeMap<&'static str, FolderAfter<'_>>,
    ) -> Result<(), BoardStoreError> {
        let entries = self.entries(written);
        // The other folders count towards what a read takes.
        let all = entries + folders.values().map(FolderAfter::entries).sum::<usize>();
        match (entries <= cap, all <= MAX_BOARD_ENTRIES) {
            (true, true) => Ok(()),
            (false, _) => Err(BoardStoreError::InvalidWrite(format!(
                "the board is full: it holds at most {cap} task files and files set aside, and \
                 would hold {entries}"
            ))),
            (true, false) => Err(BoardStoreError::InvalidWrite(format!(
                "the board is full: it holds at most {MAX_BOARD_ENTRIES} entries in all its \
                 folders, and would hold {all}"
            ))),
        }
    }

    fn bytes_within(
        &self,
        written: &[(&Task, Vec<u8>)],
        limits: Limits,
        folders: &BTreeMap<&'static str, FolderAfter<'_>>,
    ) -> Result<(), BoardStoreError> {
        let replaced: BTreeSet<String> = written
            .iter()
            .map(|(task, _)| task_file(&task.fields().id))
            .collect();
        let carried = self
            .read_bytes
            .iter()
            .filter(|(name, _)| !replaced.contains(*name));
        let tasks = carried.map(|(_, size)| size).sum::<usize>()
            + written.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        let bytes = tasks + folders.values().map(FolderAfter::bytes).sum::<usize>();
        match (tasks <= limits.tasks_bytes, bytes <= limits.board_bytes) {
            (true, true) => Ok(()),
            (false, _) => Err(BoardStoreError::InvalidWrite(format!(
                "the board would hold {tasks} bytes of task files; a read takes at most {}",
                limits.tasks_bytes
            ))),
            (true, false) => Err(BoardStoreError::InvalidWrite(format!(
                "the board would hold {bytes} bytes of files a read reads; a read takes at most {}",
                limits.board_bytes
            ))),
        }
    }
}

/// Each folder besides `tasks/`, as a write leaves it, keeps within the
/// entries and file bytes a read takes of it.
///
/// # Errors
/// An invalid write past either bound of a folder.
fn folders_within(
    folders: &BTreeMap<&'static str, FolderAfter<'_>>,
) -> Result<(), BoardStoreError> {
    for folder in &FOLDERS {
        let (name, noun) = (folder.naming.folder, folder.naming.noun);
        let Some(after) = folders.get(name) else {
            continue;
        };
        let (entries, bytes) = (after.entries(), after.bytes());
        match (entries <= folder.max_entries, bytes <= folder.max_bytes) {
            (true, true) => {}
            (false, _) => {
                return Err(BoardStoreError::InvalidWrite(format!(
                    "the board is full: its {name}/ holds at most {} {noun} files and files set \
                     aside, and would hold {entries}",
                    folder.max_entries
                )));
            }
            (true, false) => {
                return Err(BoardStoreError::InvalidWrite(format!(
                    "the board would hold {bytes} bytes of {noun} files; a read takes at most {}",
                    folder.max_bytes
                )));
            }
        }
    }
    Ok(())
}

/// A tree of `entries` fits what a read takes, or it is an invalid write,
/// so no write makes a board no one can read.
pub fn within(entries: &BTreeMap<String, String>, limits: Limits) -> Result<(), BoardStoreError> {
    match tree_bytes(entries) {
        bytes if bytes <= limits.tree_bytes => Ok(()),
        bytes => Err(BoardStoreError::InvalidWrite(format!(
            "the board would hold a tree of {bytes} bytes; a read takes at most {}",
            limits.tree_bytes
        ))),
    }
}

/// The bytes git takes to hold a tree of `entries` (name to `<mode> <type>
/// <oid>`): each is its mode without leading zeros, a space, its name, a
/// NUL and its raw object name.
fn tree_bytes(entries: &BTreeMap<String, String>) -> usize {
    entries
        .iter()
        .map(|(name, meta)| {
            let mut fields = meta.split(' ');
            let mode = fields.next().unwrap_or("").trim_start_matches('0');
            let oid = fields.nth(1).unwrap_or("");
            mode.len() + 1 + name.len() + 1 + oid.len() / 2
        })
        .sum()
}
