//! An in-memory [`ProjectBoardStore`] for tests (#2476): clients made with
//! [`MemoryBoardStore::client`] share one board, so one can move it under
//! another; [`MemoryBoardStore::before_next_write`] runs another writer's
//! change just before a write is judged, and
//! [`MemoryBoardStore::refuse_next_write`] makes the store refuse one.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, ProjectBoardStore,
    QuarantinedFile,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;

type Interleaved = Box<dyn FnOnce(&MemoryBoardStore) + Send>;

#[derive(Default)]
struct Board {
    /// `None` until bootstrapped.
    head: Option<u64>,
    tasks: BTreeMap<String, Task>,
    quarantined: Vec<QuarantinedFile>,
    interleaved: Vec<Interleaved>,
    refusal: Option<String>,
    writes: usize,
}

#[derive(Clone)]
pub struct MemoryBoardStore {
    board: Arc<Mutex<Board>>,
    writer: Identity,
}

impl Default for MemoryBoardStore {
    fn default() -> Self {
        Self {
            board: Arc::default(),
            writer: Identity {
                name: "Ada".into(),
                email: "ada@example.com".into(),
            },
        }
    }
}

impl MemoryBoardStore {
    /// Another client of the same board, writing as the same committer.
    pub fn client(&self) -> Self {
        self.clone()
    }

    /// Another client of the same board, writing as `writer`.
    pub fn client_as(&self, writer: Identity) -> Self {
        Self {
            board: self.board.clone(),
            writer,
        }
    }

    /// Runs `change` (another writer's, through this store) just before
    /// the next write is judged.
    pub fn before_next_write(&self, change: impl FnOnce(&MemoryBoardStore) + Send + 'static) {
        self.lock().interleaved.push(Box::new(change));
    }

    /// The next write is refused with `reason`.
    pub fn refuse_next_write(&self, reason: &str) {
        self.lock().refusal = Some(reason.into());
    }

    /// Sets the file `tasks/<id>.json` aside, as a store that found it
    /// unreadable does: its task is no longer read.
    pub fn quarantine(&self, id: &str, reason: &str) {
        let mut board = self.lock();
        board.tasks.remove(id);
        let id = Slug::parse(id).ok();
        let path = format!("tasks/{}.json", id.as_ref().map_or("?", Slug::as_str));
        board.quarantined.push(QuarantinedFile {
            path,
            id,
            reason: reason.into(),
        });
    }

    /// The committer of the board at `head`, when a write made it.
    pub fn committed_by(&self, _head: &BoardHead) -> Option<Identity> {
        None
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

/// One or more tasks, each id once, and a message of one short line.
fn checked(tasks: &[Task], message: &str) -> Result<(), BoardStoreError> {
    let ids: BTreeSet<&Slug> = tasks.iter().map(|task| &task.fields().id).collect();
    let message = line("message", message, 200)
        .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()));
    match (tasks.is_empty(), ids.len() == tasks.len()) {
        (false, true) => message,
        (true, _) => Err(BoardStoreError::InvalidWrite(
            "a write holds one or more tasks".into(),
        )),
        (false, false) => Err(BoardStoreError::InvalidWrite(
            "a write holds each task once".into(),
        )),
    }
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
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        checked(tasks, message)?;
        let interleaved = std::mem::take(&mut self.lock().interleaved);
        interleaved.into_iter().for_each(|change| change(self));
        let mut board = self.lock();
        let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
        if let Some(reason) = board.refusal.take() {
            return Ok(BoardWrite::Refused(reason));
        }
        if *expected != head(revision) {
            return Ok(BoardWrite::Lost);
        }
        let set_aside = |task: &Task| {
            board
                .quarantined
                .iter()
                .any(|file| file.id.as_ref() == Some(&task.fields().id))
        };
        if let Some(task) = tasks.iter().find(|task| set_aside(task)) {
            return Ok(BoardWrite::Refused(format!(
                "tasks/{}.json is quarantined",
                task.fields().id.as_str()
            )));
        }
        for task in tasks {
            board
                .tasks
                .insert(task.fields().id.as_str().to_owned(), task.clone());
        }
        board.head = Some(revision + 1);
        board.writes += 1;
        Ok(BoardWrite::Written(head(revision + 1)))
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        let mut board = self.lock();
        match board.head {
            Some(revision) => Ok(BoardBootstrap::Existing(head(revision))),
            None => {
                board.head = Some(1);
                Ok(BoardBootstrap::Created(head(1)))
            }
        }
    }

    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}
