//! SPIKE (board service): reading a board's history back: its epochs,
//! revisions and the files at each, its writers, and an online backup.
use super::*;

impl SqliteBoard {
    /// Every epoch, oldest first: its number, the revision it began at,
    /// and why.
    ///
    /// # Errors
    /// SQLite's.
    pub fn epochs(&self) -> Result<Vec<(u64, u64, String)>, BoardStoreError> {
        self.with_reader(|connection| epochs_in(connection))
    }

    /// Records a primary's epoch as it began there (a follower's copy).
    ///
    /// # Errors
    /// SQLite's.
    pub fn adopt_epoch(&self, epoch: u64, from: u64, reason: &str) -> Result<(), BoardStoreError> {
        let writer = lock(&self.writer);
        writer
            .execute(
                "INSERT OR IGNORE INTO epochs(epoch, from_revision, reason, at) VALUES (?1, ?2, ?3, ?4)",
                params![epoch, from, reason, now_text()],
            )
            .map_err(sql)?;
        drop(writer);
        *lock(&self.current) = None;
        self.committed(from);
        Ok(())
    }

    /// The revisions after `after`, at most `limit`.
    ///
    /// # Errors
    /// SQLite's.
    pub fn revisions_after(
        &self,
        after: u64,
        limit: u64,
    ) -> Result<Vec<RevisionRow>, BoardStoreError> {
        self.with_reader(|connection| {
            let mut statement = connection.prepare_cached(
                "SELECT revision, epoch, write_id, writer_name, writer_email, principal, message, kind, \
                 server_time, git_commit, git_meta FROM revisions WHERE revision > ?1 ORDER BY revision LIMIT ?2",
            )?;
            let rows = statement
                .query_map(params![after, limit], revision_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Revision `revision` and the files it changed.
    ///
    /// # Errors
    /// SQLite's; an unknown revision.
    pub fn revision(
        &self,
        revision: u64,
    ) -> Result<(RevisionRow, Vec<FileChange>), BoardStoreError> {
        self.with_reader(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let row = tx.query_row(
                "SELECT revision, epoch, write_id, writer_name, writer_email, principal, message, kind, \
                 server_time, git_commit, git_meta FROM revisions WHERE revision = ?1",
                [revision],
                revision_row,
            )?;
            let mut changes: BTreeMap<String, FileChange> = BTreeMap::new();
            {
                let mut removed = tx.prepare("SELECT path, mode FROM files WHERE removed = ?1")?;
                for item in removed.query_map([revision], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                    let (path, mode) = item?;
                    changes.insert(path.clone(), FileChange { path, mode, bytes: None });
                }
                let mut added = tx.prepare("SELECT path, mode, bytes FROM files WHERE added = ?1")?;
                for item in added.query_map([revision], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?))
                })? {
                    let (path, mode, bytes) = item?;
                    changes.insert(path.clone(), FileChange { path, mode, bytes: Some(bytes) });
                }
            }
            Ok((row, changes.into_values().collect()))
        })
    }

    /// Every file of the board at `revision`.
    ///
    /// # Errors
    /// SQLite's.
    pub fn files_at(&self, revision: u64) -> Result<Vec<BoardFile>, BoardStoreError> {
        self.with_reader(|connection| {
            let mut statement = connection.prepare_cached(
                "SELECT path, mode, bytes FROM files WHERE added <= ?1 AND (removed IS NULL OR removed > ?1) ORDER BY path",
            )?;
            let rows = statement
                .query_map([revision], |row| {
                    Ok(BoardFile { path: row.get(0)?, mode: row.get(1)?, bytes: row.get(2)? })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// The writer recorded for `revision`.
    ///
    /// # Errors
    /// SQLite's.
    pub fn writer_of(&self, revision: u64) -> Result<Option<Identity>, BoardStoreError> {
        self.with_reader(|connection| {
            connection
                .query_row(
                    "SELECT writer_name, writer_email FROM revisions WHERE revision = ?1",
                    [revision],
                    |row| {
                        Ok(Identity {
                            name: row.get(0)?,
                            email: row.get(1)?,
                        })
                    },
                )
                .optional()
        })
    }

    /// An online backup of the database to `to` (SQLite's backup API),
    /// safe while writes go on.
    ///
    /// # Errors
    /// SQLite's.
    pub fn backup(&self, to: &Path) -> Result<(), BoardStoreError> {
        self.with_reader(|connection| {
            let mut target = Connection::open(to)?;
            let backup = rusqlite::backup::Backup::new(connection, &mut target)?;
            backup.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
            Ok(())
        })
    }
}
