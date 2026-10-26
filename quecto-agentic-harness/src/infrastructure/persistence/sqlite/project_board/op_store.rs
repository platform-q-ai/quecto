//! A board server's store (epic #2705 B7): one write connection, used only
//! under the write lock in one `BEGIN IMMEDIATE` transaction (each store
//! call a savepoint of it); reads on their own, seeing only commits.
use super::*;

pub struct SqliteBoardOps {
    writes: SqliteProjectBoardStore,
    pub(super) reads: Arc<SqliteProjectBoardStore>,
    /// The write lock; it holds the last revision published to the feed.
    lock: Mutex<u64>,
    /// The change feed's notifier (B10).
    pub(super) feed: feed::FeedHub,
    /// The board this file is bound to, once [`Self::bind`] binds it.
    board: std::sync::OnceLock<String>,
    /// The board file's path, for a follower's diverged mark beside it.
    pub(super) path: std::path::PathBuf,
}

/// The write lock and the write connection, held together.
pub(super) type Applying<'a> = (
    std::sync::MutexGuard<'a, u64>,
    std::sync::MutexGuard<'a, Connection>,
);

/// Binds an unbound board (#2741); a bound one is left as it is.
const BIND: &str = "UPDATE history_meta SET board_id=?1 WHERE id=1 AND board_id IS NULL";

/// A served board file's binding to a board id (#2741).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardBinding {
    /// Bound to the server's board, or holding no board yet.
    Bound,
    /// Bound to this other board.
    Other(String),
}

impl SqliteBoardOps {
    /// The board at `path`, read as `reader` (a read never writes).
    /// # Errors
    /// As [`SqliteProjectBoardStore::open`], for either connection.
    pub fn open(path: &Path, reader: Identity) -> Result<Self, BoardStoreError> {
        let reads = SqliteProjectBoardStore::open(path, reader.clone())?;
        let published = {
            let c = reads.connection.lock().map_err(unavailable)?;
            feed::tip(&c)?.revision
        };
        Ok(Self {
            writes: SqliteProjectBoardStore::open(path, reader)?,
            reads: Arc::new(reads),
            lock: Mutex::new(published),
            feed: feed::FeedHub::default(),
            board: std::sync::OnceLock::new(),
            path: path.to_path_buf(),
        })
    }

    /// Binds this file to `board` (#2741): an unbound board is bound now,
    /// and one first made by a write here is bound by that write. A file
    /// bound to another board is left so, and named.
    /// # Errors
    /// The file cannot be read or written.
    pub fn bind(&self, board: &str) -> Result<BoardBinding, BoardStoreError> {
        let _held = self.lock.lock().map_err(unavailable)?;
        let c = self.locked()?;
        c.execute(BIND, [board]).map_err(unavailable)?;
        let bound: Option<Option<String>> = c
            .query_row("SELECT board_id FROM history_meta WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(unavailable)?;
        match bound.flatten() {
            None => {}
            Some(id) if id == board => {}
            Some(other) => return Ok(BoardBinding::Other(other)),
        }
        let set = self.board.get_or_init(|| board.to_owned());
        assert_eq!(set, board, "a board file is bound to one board");
        Ok(BoardBinding::Bound)
    }

    /// Inside the transaction: the kept answer, or `op`'s, kept.
    fn ran(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        match storage::kept_answer(&*self.locked()?, write)? {
            Some(Some(answer)) => return Ok(Transacted::Replayed(answer)),
            Some(None) => return Ok(Transacted::Reused),
            None => {}
        }
        let bound = SqliteProjectBoardStore {
            connection: Arc::clone(&self.writes.connection),
            writer: write.committer.clone(),
            principal: Some(write.principal.clone()),
            #[cfg(feature = "test-support")]
            payload_fetches: Mutex::new(vec![]),
            #[cfg(feature = "test-support")]
            hooks: Arc::clone(&self.writes.hooks),
        };
        let answer = op(Arc::new(bound))?;
        // A board first made by this write is bound with it.
        if let Some(board) = self.board.get() {
            self.locked()?.execute(BIND, [board]).map_err(unavailable)?;
        }
        storage::keep_answer(&*self.locked()?, write, &answer)?;
        Ok(Transacted::Answered(answer))
    }

    /// Publishes the revisions committed after `published` (read on the
    /// write connection, `c`, under the write lock), which becomes the
    /// last one. One that cannot be read resets every subscriber, which
    /// then resumes from the store: never a silent gap.
    pub(super) fn publish(&self, c: &Connection, published: &mut u64) {
        match feed::after(c, *published, MAX_REVISION_BATCH) {
            Ok(revisions) => {
                if let Some(last) = revisions.last() {
                    *published = last.revision;
                    self.feed.publish(&revisions);
                }
            }
            Err(_) => {
                if let Ok(tip) = feed::tip(c) {
                    *published = tip.revision;
                }
                self.feed.reset_all();
            }
        }
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Connection>, BoardStoreError> {
        self.writes.connection.lock().map_err(unavailable)
    }

    /// The write lock (the last revision published) and the write
    /// connection, for a follower's batch (B11).
    pub(super) fn locked_for_apply(&self) -> Result<Applying<'_>, BoardStoreError> {
        let published = self.lock.lock().map_err(unavailable)?;
        Ok((published, self.locked()?))
    }

    /// The board this file is bound to, once bound.
    pub(super) fn bound(&self) -> Option<&str> {
        self.board.get().map(String::as_str)
    }
}

/// A follower's operations store (B11): it reads its board, and refuses
/// every write outright, so no op (even one mis-read as a read) can fork
/// its copy: only its primary's batches change it.
pub struct StandbyOps(Arc<SqliteBoardOps>);

impl SqliteBoardOps {
    /// This store as a follower's operations store.
    pub fn standby(self: &Arc<Self>) -> StandbyOps {
        StandbyOps(Arc::clone(self))
    }
}

impl BoardOpStore for StandbyOps {
    fn reads(&self) -> Arc<dyn ProjectBoardStore> {
        self.0.reads()
    }

    fn kept(&self, write: &OpWrite) -> Result<Option<Transacted>, BoardStoreError> {
        self.0.kept(write)
    }

    fn transact(
        &self,
        _: &OpWrite,
        _: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        Err(BoardStoreError::Unavailable(
            "this server is a standby: it writes nothing but its primary's copy".into(),
        ))
    }
}

impl BoardOpStore for SqliteBoardOps {
    fn reads(&self) -> Arc<dyn ProjectBoardStore> {
        Arc::clone(&self.reads) as _
    }

    fn kept(&self, write: &OpWrite) -> Result<Option<Transacted>, BoardStoreError> {
        let c = self.reads.connection.lock().map_err(unavailable)?;
        Ok(storage::kept_answer(&c, write)?.map(|kept| match kept {
            Some(answer) => Transacted::Replayed(answer),
            None => Transacted::Reused,
        }))
    }

    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        let mut published = self.lock.lock().map_err(unavailable)?;
        self.locked()?
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(unavailable)?;
        let ran = self.ran(write, op);
        // Kept or replayed: commit. Failed: undo every write. A commit that
        // fails is undone too, so no transaction is ever left open.
        let c = self.locked()?;
        let ended = match ran.is_ok() {
            true => c.execute_batch("COMMIT"),
            false => c.execute_batch("ROLLBACK"),
        };
        match c.is_autocommit() {
            true => {}
            false => drop(c.execute_batch("ROLLBACK")),
        }
        let transacted = ran?;
        ended.map_err(unavailable)?;
        // Committed: its new revisions go to the feed, still under the
        // lock, so the feed tells them in the order they were made.
        if let Transacted::Answered(_) = transacted {
            self.publish(&c, &mut published);
        }
        Ok(transacted)
    }
}

#[cfg(test)]
#[path = "op_store_tests.rs"]
mod tests;
