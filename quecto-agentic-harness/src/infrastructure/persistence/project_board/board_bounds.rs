//! The bounds a change to the board keeps (`board_tree`): the entries of
//! its `tasks/` tree a write leaves, which a clear never grows, and the
//! task-file bytes a read reads.
use super::{Board, Limits, MAX_TASK_ENTRIES, task_file};
use crate::application::project_board::ports::BoardStoreError;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use std::collections::BTreeSet;

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
        match entries <= cap {
            true => Ok(()),
            false => Err(BoardStoreError::InvalidWrite(format!(
                "the board would hold {entries} task files and files set aside; it holds at \
                 most {cap}"
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
        let bytes = carried.map(|(_, size)| size).sum::<usize>()
            + written.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
        match bytes <= limits.board_bytes {
            true => Ok(()),
            false => Err(BoardStoreError::InvalidWrite(format!(
                "the board would hold {bytes} bytes of task files; a read takes at most {}",
                limits.board_bytes
            ))),
        }
    }
}
