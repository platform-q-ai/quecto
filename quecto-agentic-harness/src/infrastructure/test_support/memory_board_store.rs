//! An in-memory [`ProjectBoardStore`] for tests (#2476): clients made with
//! [`MemoryBoardStore::client`] share one board, so one can move it under
//! another; [`MemoryBoardStore::before_next_write`] runs another writer's
//! change just before a write is judged, and
//! [`MemoryBoardStore::refuse_next_write`] makes the store refuse one.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError,
    BoardWrite, KeepCopy, ProjectBoardStore, QuarantinedFile, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::schema_error::line;
use crate::domain::project_board::value_objects::slug::Slug;

type Interleaved = Box<dyn FnOnce(&MemoryBoardStore) + Send>;

#[derive(Default)]
struct Board {
    /// `None` until bootstrapped.
    head: Option<u64>,
    /// Every artifact of every kind, by its key.
    artifacts: BTreeMap<ArtifactKey, BoardArtifact>,
    quarantined: Vec<QuarantinedFile>,
    interleaved: Vec<Interleaved>,
    after_read: Vec<Interleaved>,
    refusal: Option<String>,
    writes: usize,
    /// Who made the board at each revision a write made.
    committers: BTreeMap<u64, Identity>,
    /// The message of each revision a write made.
    messages: BTreeMap<u64, String>,
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

    /// Another client of the same board, writing as `writer`. Unlike a
    /// real store, it takes any identity, so the use cases' own check of
    /// an invalid one can be shown.
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

    /// Runs `change` (another writer's, through this store) just after the
    /// next read is answered: the board moves on from the snapshot the
    /// reader holds.
    pub fn after_next_read(&self, change: impl FnOnce(&MemoryBoardStore) + Send + 'static) {
        self.lock().after_read.push(Box::new(change));
    }

    /// The next write is refused with `reason`.
    pub fn refuse_next_write(&self, reason: &str) {
        self.lock().refusal = Some(reason.into());
    }

    /// Sets the file `tasks/<id>.md` aside, as a store that found it
    /// unreadable does: its task is no longer read. Spoiling the file is a
    /// change to the board, so the head moves, as a commit moves the git
    /// store's.
    ///
    /// # Panics
    /// Before the board is bootstrapped: there is no file to spoil.
    pub fn quarantine(&self, id: &str, reason: &str) {
        let mut board = self.lock();
        let revision = board
            .head
            .expect("a bootstrapped board to quarantine a file of");
        board.head = Some(revision + 1);
        let id = Slug::parse(id).ok();
        let path = format!("tasks/{}.md", id.as_ref().map_or("?", Slug::as_str));
        let key = id.map(ArtifactKey::Task);
        if let Some(key) = &key {
            board.artifacts.remove(key);
        }
        board.quarantined.push(QuarantinedFile {
            path,
            key,
            reason: reason.into(),
            object: format!("{:040x}", revision + 1),
            carried: true,
        });
    }

    /// Sets the file of the artifact `key` aside (at `path`), as a store
    /// that found it unreadable does: it is no longer read, and every write
    /// carries it forward. The head moves.
    ///
    /// # Panics
    /// Before the board is bootstrapped.
    pub fn quarantine_key(&self, key: ArtifactKey, path: &str, reason: &str) {
        let mut board = self.lock();
        let revision = board.head.expect("a bootstrapped board");
        board.head = Some(revision + 1);
        board.artifacts.remove(&key);
        board.quarantined.push(QuarantinedFile {
            path: path.into(),
            key: Some(key),
            reason: reason.into(),
            object: format!("{:040x}", revision + 1),
            carried: true,
        });
    }

    /// Sets aside a task file at `path` whose name spells no task id, as
    /// a commit adding it does: the head moves, and every write carries it
    /// forward.
    ///
    /// # Panics
    /// Before the board is bootstrapped.
    pub fn set_aside_path(&self, path: &str, reason: &str) {
        self.set_aside(path, reason, true);
    }

    /// Sets aside a stray at `path` (not a task file), as a commit adding
    /// it does: the head moves, and the next write drops it.
    ///
    /// # Panics
    /// Before the board is bootstrapped.
    pub fn set_aside_stray(&self, path: &str, reason: &str) {
        self.set_aside(path, reason, false);
    }

    fn set_aside(&self, path: &str, reason: &str, carried: bool) {
        let mut board = self.lock();
        let revision = board.head.expect("a bootstrapped board");
        board.head = Some(revision + 1);
        board.quarantined.push(QuarantinedFile {
            path: path.into(),
            key: None,
            reason: reason.into(),
            object: format!("{:040x}", revision + 1),
            carried,
        });
    }

    /// Deletes the quarantined file of `id`, as a commit removing it
    /// does: the head moves, and tasks naming `id` now name no task.
    ///
    /// # Panics
    /// When no quarantined file has that id.
    pub fn delete_quarantined(&self, id: &str) {
        let mut board = self.lock();
        let before = board.quarantined.len();
        board
            .quarantined
            .retain(|file| file.id().map(Slug::as_str) != Some(id));
        assert_eq!(
            board.quarantined.len() + 1,
            before,
            "one file of id {id} is quarantined"
        );
        let revision = board.head.expect("a bootstrapped board");
        board.head = Some(revision + 1);
    }

    /// Lists the quarantined file of `id` twice, same path and object, as
    /// a git board shows two names that differ only in a character no one
    /// sees.
    ///
    /// # Panics
    /// When no quarantined file has that id.
    pub fn show_twice(&self, id: &str) {
        let mut board = self.lock();
        let file = board
            .quarantined
            .iter()
            .find(|file| file.id().map(Slug::as_str) == Some(id))
            .cloned()
            .expect("a quarantined file of that id");
        board.quarantined.push(file);
    }

    /// The message of the change that made the board at `head`, when a
    /// write made it.
    pub fn message_at(&self, at: &BoardHead) -> Option<String> {
        let board = self.lock();
        board
            .messages
            .iter()
            .find(|(revision, _)| head(**revision) == *at)
            .map(|(_, message)| message.clone())
    }

    /// The committer of the board at `head`, when a write made it.
    pub fn committed_by(&self, at: &BoardHead) -> Option<Identity> {
        let board = self.lock();
        board
            .committers
            .iter()
            .find(|(revision, _)| head(**revision) == *at)
            .map(|(_, committer)| committer.clone())
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

/// One or more artifacts, each key once, and a message of one short line.
fn checked(artifacts: &[BoardArtifact], message: &str) -> Result<(), BoardStoreError> {
    let keys: BTreeSet<ArtifactKey> = artifacts.iter().map(BoardArtifact::key).collect();
    let message = line("message", message, 200)
        .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()));
    match (artifacts.is_empty(), keys.len() == artifacts.len()) {
        (false, true) => message,
        (true, _) => Err(BoardStoreError::InvalidWrite(
            "a write holds one or more artifacts".into(),
        )),
        (false, false) => Err(BoardStoreError::InvalidWrite(
            "a write holds each artifact once".into(),
        )),
    }
}

/// Every artifact of `artifacts` that `pick` takes, in key order.
fn of_kind<'a, T: Clone + 'a>(
    artifacts: &'a BTreeMap<ArtifactKey, BoardArtifact>,
    pick: impl Fn(&'a BoardArtifact) -> Option<&'a T>,
) -> Vec<T> {
    artifacts.values().filter_map(pick).cloned().collect()
}

impl ProjectBoardStore for MemoryBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        let (snapshot, after) = {
            let mut board = self.lock();
            let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
            let snapshot = BoardSnapshot {
                head: head(revision),
                tasks: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Task(task) => Some(task),
                    _ => None,
                }),
                plans: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Plan(plan) => Some(plan),
                    _ => None,
                }),
                runs: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Run(run) => Some(run),
                    _ => None,
                }),
                changes: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Change(change) => Some(change),
                    _ => None,
                }),
                reviews: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Review(review) => Some(review),
                    _ => None,
                }),
                discussions: of_kind(&board.artifacts, |artifact| match artifact {
                    BoardArtifact::Discussion(discussion) => Some(discussion),
                    _ => None,
                }),
                quarantined: board.quarantined.clone(),
            };
            (snapshot, std::mem::take(&mut board.after_read))
        };
        after.into_iter().for_each(|change| change(self));
        Ok(snapshot.asserted())
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        checked(artifacts, message)?;
        let interleaved = std::mem::take(&mut self.lock().interleaved);
        interleaved.into_iter().for_each(|change| change(self));
        let mut board = self.lock();
        let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
        // As a git store judges: a stale head is lost before any refusal.
        if *expected != head(revision) {
            return Ok(BoardWrite::Lost);
        }
        if let Some(reason) = board.refusal.take() {
            return Ok(BoardWrite::Refused(reason));
        }
        let set_aside = |key: &ArtifactKey| {
            board
                .quarantined
                .iter()
                .find(|file| file.key.as_ref() == Some(key))
                .map(|file| file.path.clone())
        };
        if let Some(path) = artifacts
            .iter()
            .find_map(|artifact| set_aside(&artifact.key()))
        {
            return Ok(BoardWrite::Refused(format!("{path} is quarantined")));
        }
        for artifact in artifacts {
            board.artifacts.insert(artifact.key(), artifact.clone());
        }
        // As a git write: a stray set aside is dropped, a carried file kept.
        board.quarantined.retain(|file| file.carried);
        board.head = Some(revision + 1);
        board.writes += 1;
        board.committers.insert(revision + 1, self.writer.clone());
        board.messages.insert(revision + 1, message.to_string());
        Ok(BoardWrite::Written(head(revision + 1)))
    }

    /// A name is never listed twice in memory, so a clear that resolves a
    /// name listed twice is refused.
    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let path = clear.path.as_str();
        line("message", message, 200)
            .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        // An object id as git spells one: lowercase hex, SHA-1 or SHA-256.
        let hex = |text: &str| {
            matches!(text.len(), 40 | 64)
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let kept = clear.repeated.values().flat_map(|keep| match keep {
            KeepCopy::Object { keep, seen } => std::iter::once(keep).chain(seen).collect(),
            KeepCopy::Drop(seen) => seen.iter().collect::<Vec<_>>(),
        });
        if let Some(oid) = std::iter::once(&clear.object)
            .chain(kept)
            .find(|oid| !hex(oid))
        {
            return Err(BoardStoreError::InvalidWrite(format!(
                "{oid:?} is not an object id"
            )));
        }
        let interleaved = std::mem::take(&mut self.lock().interleaved);
        interleaved.into_iter().for_each(|change| change(self));
        let mut board = self.lock();
        let revision = board.head.ok_or(BoardStoreError::NoBoard)?;
        // As a git store judges: a stale head is lost before any refusal.
        if *expected != head(revision) {
            return Ok(BoardWrite::Lost);
        }
        if let Some(reason) = board.refusal.take() {
            return Ok(BoardWrite::Refused(reason));
        }
        if let Some(path) = clear.repeated.keys().next() {
            return Ok(BoardWrite::Refused(format!(
                "{path:?} is not listed more than once"
            )));
        }
        let named = |file: &QuarantinedFile| file.path == path && file.object == clear.object;
        // Entries that share path and object are one file, as on a git
        // board.
        match board.quarantined.iter().any(named) {
            true => {}
            false => return Ok(BoardWrite::Refused(format!("{path:?} is not set aside"))),
        }
        board.quarantined.retain(|file| !named(file));
        // A clear is a write: a stray set aside is dropped with it.
        board.quarantined.retain(|file| file.carried);
        board.head = Some(revision + 1);
        board.writes += 1;
        board.committers.insert(revision + 1, self.writer.clone());
        board.messages.insert(revision + 1, message.to_string());
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
