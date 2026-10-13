//! The bounds a change to the board keeps (`board_tree`): the entries of
//! its `tasks/` tree a write leaves, which a clear never grows, and, with
//! the other folders a write carries as they are (#2481 S9), the entries
//! and file bytes a read reads.
use super::{Board, Limits, MAX_BOARD_ENTRIES, MAX_TASK_ENTRIES, Unread, task_file, too_large};
use crate::application::project_board::ports::BoardStoreError;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::{BTreeMap, BTreeSet};

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
    /// The board, with `written` in place, keeps within what a write
    /// leaves: the entries of its `tasks/` tree (each task one file, each
    /// file set aside one), at most [`MAX_TASK_ENTRIES`], and the task-file
    /// bytes a read reads. Every write checks it before it builds anything.
    ///
    /// # Errors
    /// An invalid write past either bound.
    pub fn within_bounds(
        &self,
        written: &[(&Task, Vec<u8>)],
        limits: Limits,
    ) -> Result<(), BoardStoreError> {
        self.entries_within(written, MAX_TASK_ENTRIES)?;
        self.bytes_within(written, limits)
    }

    /// The board a clear of `before` leaves keeps within what a read takes:
    /// a clear never adds an entry, so a board read past what a write
    /// leaves (one written before the task files became markdown) can
    /// still be cleared, file by file, back within it.
    ///
    /// # Errors
    /// An invalid clear past either bound.
    pub fn within_clear_bounds(
        &self,
        before: &Board,
        limits: Limits,
    ) -> Result<(), BoardStoreError> {
        let cap = MAX_TASK_ENTRIES.max(before.entries(&[]));
        self.entries_within(&[], cap)?;
        self.bytes_within(&[], limits)
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
    ) -> Result<(), BoardStoreError> {
        let entries = self.entries(written);
        // The folders a write carries count towards what a read takes.
        let all = entries + self.carried_entries();
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
        let bytes = tasks + self.carried_bytes();
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
