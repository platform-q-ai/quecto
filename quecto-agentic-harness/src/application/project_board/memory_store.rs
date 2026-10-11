//! An in-memory [`ProjectBoardStore`] for tests (#2476): clients made with
//! [`MemoryBoardStore::client`] share one board, so one can move it under
//! another, and [`MemoryBoardStore::before_next_write`] runs another
//! writer's change just before a write is judged.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::ports::{
    BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, Bootstrapped, ProjectBoardStore,
    QuarantinedFile,
};
use crate::domain::project_board::entities::task::Task;

type Interleaved = Box<dyn FnOnce(&MemoryBoardStore) + Send>;

#[derive(Default)]
struct Board {
    /// `None` until bootstrapped.
    head: Option<u64>,
    tasks: BTreeMap<String, Task>,
    quarantined: Vec<QuarantinedFile>,
    interleaved: Vec<Interleaved>,
    writes: usize,
}

#[derive(Clone, Default)]
pub struct MemoryBoardStore {
    board: Arc<Mutex<Board>>,
}

impl MemoryBoardStore {
    /// Another client of the same board.
    pub fn client(&self) -> Self {
        self.clone()
    }

    /// Runs `change` (another writer's, through this store) just before
    /// the next write is judged.
    pub fn before_next_write(&self, change: impl FnOnce(&MemoryBoardStore) + Send + 'static) {
        self.lock().interleaved.push(Box::new(change));
    }

    /// Sets a file aside, as a store that found it unreadable does.
    pub fn quarantine(&self, path: &str, reason: &str) {
        self.lock().quarantined.push(QuarantinedFile {
            path: path.into(),
            reason: reason.into(),
        });
    }

    /// The writes the board accepted.
    pub fn writes(&self) -> usize {
        self.lock().writes
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Board> {
        self.board
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn head(revision: u64) -> BoardHead {
    BoardHead(format!("r{revision}"))
}

impl ProjectBoardStore for MemoryBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        let board = self.lock();
        let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
        Ok(BoardSnapshot {
            head: head(revision),
            tasks: board.tasks.values().cloned().collect(),
            quarantined: board.quarantined.clone(),
        })
    }

    fn write(
        &self,
        expected: &BoardHead,
        tasks: &[Task],
        _message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let interleaved = std::mem::take(&mut self.lock().interleaved);
        interleaved.into_iter().for_each(|change| change(self));
        let mut board = self.lock();
        let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
        let _ = expected;
        for task in tasks {
            board
                .tasks
                .insert(task.fields().id.as_str().to_owned(), task.clone());
        }
        board.head = Some(revision + 1);
        board.writes += 1;
        Ok(BoardWrite::Written(head(revision + 1)))
    }

    fn bootstrap(&self) -> Result<Bootstrapped, BoardStoreError> {
        let mut board = self.lock();
        match board.head {
            Some(revision) => Ok(Bootstrapped::Existing(head(revision))),
            None => {
                board.head = Some(1);
                Ok(Bootstrapped::Created(head(1)))
            }
        }
    }
}
