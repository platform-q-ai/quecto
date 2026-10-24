//! A board server's store, written one operation at a time (epic #2705
//! B7): one connection that only an operation holding the board's write
//! lock uses, in one `BEGIN IMMEDIATE` transaction, each store call in a
//! savepoint of it. Reads have their own connection, which sees only what
//! has committed.
use super::*;

pub struct SqliteBoardOps {
    writes: SqliteProjectBoardStore,
    reads: Arc<SqliteProjectBoardStore>,
    lock: Mutex<()>,
}

impl SqliteBoardOps {
    /// The board at `path`, read as `reader` (a read never writes).
    /// # Errors
    /// As [`SqliteProjectBoardStore::open`], for either connection.
    pub fn open(path: &Path, reader: Identity) -> Result<Self, BoardStoreError> {
        Ok(Self {
            writes: SqliteProjectBoardStore::open(path, reader.clone())?,
            reads: Arc::new(SqliteProjectBoardStore::open(path, reader)?),
            lock: Mutex::new(()),
        })
    }

    /// Inside the open transaction: the answer kept for the write's id,
    /// or `op`'s, run on the write connection as its committer and kept.
    fn ran(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        let id = write.write_id.as_str();
        if let Some(answer) = storage::kept_answer(&*self.locked()?, id)? {
            return Ok(Transacted::Replayed(answer));
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
        storage::keep_answer(&*self.locked()?, id, &answer)?;
        Ok(Transacted::Answered(answer))
    }

    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Connection>, BoardStoreError> {
        self.writes.connection.lock().map_err(unavailable)
    }
}

impl BoardOpStore for SqliteBoardOps {
    fn reads(&self) -> Arc<dyn ProjectBoardStore> {
        Arc::clone(&self.reads) as _
    }

    fn transact(
        &self,
        write: &OpWrite,
        op: &mut dyn FnMut(Arc<dyn ProjectBoardStore>) -> Result<String, BoardStoreError>,
    ) -> Result<Transacted, BoardStoreError> {
        let _held = self.lock.lock().map_err(unavailable)?;
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
        ended.map(|()| transacted).map_err(unavailable)
    }
}
