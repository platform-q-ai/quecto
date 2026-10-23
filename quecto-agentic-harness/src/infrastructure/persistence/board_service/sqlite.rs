//! SPIKE (board service): one board in one SQLite file (WAL,
//! `synchronous=FULL`). Files are kept as versioned rows (`added`,
//! `removed` revisions), so every revision's board and its change set can
//! be read back; `revisions` records who wrote each revision, when (the
//! server's clock), its message and the write's id, so a retried write is
//! answered once. The head is the board's `epoch:revision`: one
//! compare-and-swap for the whole board, as the git branch gives.
use super::files::{BoardFile, Head, REGULAR, Read, read_file, root_files, snapshot_from_reads};
use crate::application::project_board::ports::{
    ArtifactKey, BoardBootstrap, BoardSnapshot, BoardStoreError, BoardWrite, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::schema_error::line;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta(k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files(
  path TEXT NOT NULL, mode TEXT NOT NULL, bytes BLOB NOT NULL,
  added INTEGER NOT NULL, removed INTEGER,
  PRIMARY KEY(path, added));
CREATE INDEX IF NOT EXISTS files_live ON files(path) WHERE removed IS NULL;
CREATE INDEX IF NOT EXISTS files_removed ON files(removed);
CREATE TABLE IF NOT EXISTS revisions(
  revision INTEGER PRIMARY KEY, epoch INTEGER NOT NULL,
  write_id TEXT UNIQUE, writer_name TEXT NOT NULL, writer_email TEXT NOT NULL,
  principal TEXT, message TEXT NOT NULL, kind TEXT NOT NULL,
  server_time TEXT NOT NULL, git_commit TEXT, git_meta BLOB);
CREATE TABLE IF NOT EXISTS epochs(epoch INTEGER PRIMARY KEY, from_revision INTEGER NOT NULL,
  reason TEXT NOT NULL, at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS board_members(principal TEXT PRIMARY KEY, role TEXT NOT NULL,
  added TEXT NOT NULL);
";

/// Who made a revision, and why.
#[derive(Debug, Clone)]
pub struct RevisionMeta {
    pub write_id: Option<String>,
    pub writer: Identity,
    pub principal: Option<String>,
    pub message: String,
    pub kind: String,
    /// The git commit an imported revision was, and its raw header and
    /// message (everything but `tree` and `parent` lines), for the export.
    pub git_commit: Option<String>,
    pub git_meta: Option<Vec<u8>>,
    /// The time to record; the server's clock when `None`.
    pub server_time: Option<String>,
}

impl RevisionMeta {
    pub fn by(writer: Identity, message: &str, kind: &str) -> Self {
        Self {
            write_id: None,
            writer,
            principal: None,
            message: message.to_string(),
            kind: kind.to_string(),
            git_commit: None,
            git_meta: None,
            server_time: None,
        }
    }
}

/// A revision as the history keeps it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RevisionRow {
    pub revision: u64,
    pub epoch: u64,
    pub write_id: Option<String>,
    pub writer_name: String,
    pub writer_email: String,
    pub principal: Option<String>,
    pub message: String,
    pub kind: String,
    pub server_time: String,
    pub git_commit: Option<String>,
    #[serde(with = "opt_b64")]
    pub git_meta: Option<Vec<u8>>,
}

/// One file change of a revision: `bytes` `None` removes it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileChange {
    pub path: String,
    pub mode: String,
    #[serde(with = "opt_b64")]
    pub bytes: Option<Vec<u8>>,
}

pub mod opt_b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(bytes) => s.serialize_some(&STANDARD.encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|text| STANDARD.decode(text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// The board at its head, decoded once per head.
pub struct Current {
    pub head: Head,
    pub files: Vec<BoardFile>,
    /// Each file as read, by path: a file whose bytes did not change since
    /// the head before is not decoded again.
    pub reads: BTreeMap<String, (Vec<u8>, Read)>,
    pub snapshot: BoardSnapshot,
}

/// Whether the board takes writes now, asked inside each write's
/// transaction: a follower takes none, nor a primary whose lease lapsed.
pub type WriteGate = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

pub struct SqliteBoard {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
    current: Mutex<Option<Arc<Current>>>,
    /// A generation counted up on every change, for the feed's waiters.
    pub latest: Arc<(Mutex<u64>, Condvar)>,
    gate: Mutex<Option<WriteGate>>,
}

fn sql(error: rusqlite::Error) -> BoardStoreError {
    BoardStoreError::Unavailable(format!("the board database: {error}"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_text() -> String {
    humantime::format_rfc3339_millis(std::time::SystemTime::now()).to_string()
}

fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(10))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(connection)
}

/// What a write commits: puts and removals, by path.
type Changes = BTreeMap<String, Option<(String, Vec<u8>)>>;

impl SqliteBoard {
    /// Opens (creating) the board database at `path`.
    ///
    /// # Errors
    /// SQLite's.
    pub fn open(path: &Path) -> Result<Self, BoardStoreError> {
        let writer = connect(path).map_err(sql)?;
        writer.execute_batch(SCHEMA).map_err(sql)?;
        let revision: u64 = writer
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM revisions",
                [],
                |row| row.get(0),
            )
            .map_err(sql)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            readers: Mutex::new(vec![]),
            current: Mutex::new(None),
            latest: Arc::new((Mutex::new(revision), Condvar::new())),
            gate: Mutex::new(None),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_gate(&self, gate: Option<WriteGate>) {
        *lock(&self.gate) = gate;
    }

    /// Test hook: the next write is refused with `reason`.
    /// Kept in the database, so every client of the file sees it.
    pub fn refuse_next_write(&self, reason: &str) {
        let writer = lock(&self.writer);
        writer
            .execute(
                "INSERT OR REPLACE INTO meta(k, v) VALUES ('refuse_next', ?1)",
                [reason],
            )
            .expect("the test hook's row");
    }

    fn with_reader<T>(
        &self,
        body: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, BoardStoreError> {
        let pooled = lock(&self.readers).pop();
        let mut connection = match pooled {
            Some(connection) => connection,
            None => connect(&self.path).map_err(sql)?,
        };
        let out = body(&mut connection).map_err(sql);
        lock(&self.readers).push(connection);
        out
    }

    /// The head, or `None` before a bootstrap.
    ///
    /// # Errors
    /// SQLite's.
    pub fn head(&self) -> Result<Option<Head>, BoardStoreError> {
        self.with_reader(|connection| head_in(connection))
    }

    /// The board at its head, decoded once per head.
    ///
    /// # Errors
    /// No board, or SQLite's.
    pub fn current(&self) -> Result<Arc<Current>, BoardStoreError> {
        let (head, files) = self.with_reader(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let head = head_in(&tx)?;
            let cached = lock(&self.current).clone();
            match (head, cached) {
                (Some(head), Some(cached)) if cached.head == head => Ok((Some(head), None)),
                (Some(head), _) => Ok((Some(head), Some(live_files(&tx)?))),
                (None, _) => Ok((None, None)),
            }
        })?;
        let head = head.ok_or(BoardStoreError::NoBoard)?;
        match files {
            None => lock(&self.current)
                .clone()
                .filter(|cached| cached.head == head)
                .map_or_else(|| self.current(), Ok),
            Some(files) => {
                let before = lock(&self.current).clone();
                let reads: BTreeMap<String, (Vec<u8>, Read)> = files
                    .iter()
                    .map(|file| {
                        let kept = before
                            .as_ref()
                            .and_then(|before| before.reads.get(&file.path))
                            .filter(|(bytes, _)| *bytes == file.bytes)
                            .map(|(_, read)| read.clone());
                        let read =
                            kept.unwrap_or_else(|| read_file(&file.path, &file.mode, &file.bytes));
                        (file.path.clone(), (file.bytes.clone(), read))
                    })
                    .collect();
                let snapshot = snapshot_from_reads(
                    head.board_head(),
                    reads.values().map(|(_, read)| read.clone()).collect(),
                );
                let current = Arc::new(Current {
                    head,
                    files,
                    reads,
                    snapshot,
                });
                *lock(&self.current) = Some(Arc::clone(&current));
                Ok(current)
            }
        }
    }

    /// Bootstraps the board: revision 1 of epoch 1, the root's files.
    ///
    /// # Errors
    /// SQLite's.
    pub fn bootstrap(&self, meta: &RevisionMeta) -> Result<BoardBootstrap, BoardStoreError> {
        let mut writer = lock(&self.writer);
        let tx = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        if let Some(head) = head_in(&tx).map_err(sql)? {
            return Ok(BoardBootstrap::Existing(head.board_head()));
        }
        let head = Head {
            epoch: 1,
            revision: 1,
        };
        tx.execute(
            "INSERT INTO epochs(epoch, from_revision, reason, at) VALUES (1, 1, 'bootstrap', ?1)",
            [now_text()],
        )
        .map_err(sql)?;
        let changes: Changes = root_files()
            .into_iter()
            .map(|(path, bytes)| (path, Some((REGULAR.to_string(), bytes))))
            .collect();
        commit_in(&tx, head, &changes, meta).map_err(sql)?;
        tx.commit().map_err(sql)?;
        drop(writer);
        self.committed(head.revision);
        Ok(BoardBootstrap::Created(head.board_head()))
    }

    /// The board moved (a revision, or a new epoch): the feed's waiters
    /// wake on the generation.
    fn committed(&self, _revision: u64) {
        let (generation, waiters) = &*self.latest;
        *lock(generation) += 1;
        waiters.notify_all();
    }

    /// Writes `files` (each an artifact's file by its key, already
    /// encoded by its codec) if the board is still at `expected`.
    ///
    /// # Errors
    /// An invalid write, no board, or SQLite's.
    pub fn write_files(
        &self,
        expected: &str,
        files: &[(ArtifactKey, String, Vec<u8>)],
        meta: &RevisionMeta,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", &meta.message, 200).map_err(|error| invalid(error.to_string()))?;
        let keys: BTreeSet<&ArtifactKey> = files.iter().map(|(key, _, _)| key).collect();
        if keys.len() != files.len() || files.is_empty() {
            return Err(invalid("write one or more board files, each once".into()));
        }
        // The server judges every file by its codec, as the git store does:
        // what the file at its path reads as must be the artifact named.
        for (key, path, bytes) in files {
            match read_file(path, REGULAR, bytes) {
                Read::Artifact(artifact) if artifact.key() == *key => {}
                Read::Artifact(artifact) => {
                    return Err(invalid(format!(
                        "{path} holds {}, not {key}",
                        artifact.key()
                    )));
                }
                Read::Aside(file) => return Err(invalid(format!("{key}: {}", file.reason))),
                Read::Meta => return Err(invalid(format!("{path} is not an artifact's file"))),
            }
        }
        self.transact(expected, meta, |current| {
            let set_aside = current.snapshot.quarantined.iter().find(|file| {
                file.carried && file.key.as_ref().is_some_and(|key| keys.contains(key))
            });
            if let Some(file) = set_aside {
                return Err(format!("{} is set aside: {}", file.path, file.reason));
            }
            let mut changes = strays_dropped(current);
            for (_, path, bytes) in files {
                changes.insert(path.clone(), Some((REGULAR.to_string(), bytes.clone())));
            }
            Ok(changes)
        })
    }

    /// Removes the file set aside that `clear` names.
    ///
    /// # Errors
    /// An invalid clear, no board, or SQLite's.
    pub fn clear(
        &self,
        expected: &str,
        clear: &SetAsideClear,
        meta: &RevisionMeta,
    ) -> Result<BoardWrite, BoardStoreError> {
        let invalid = |why: String| BoardStoreError::InvalidWrite(why);
        line("message", &meta.message, 200).map_err(|error| invalid(error.to_string()))?;
        let hex = |text: &str| {
            matches!(text.len(), 40 | 64)
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !hex(&clear.object) {
            return Err(invalid(format!("{:?} is not an object id", clear.object)));
        }
        self.transact(expected, meta, |current| {
            if let Some(path) = clear.repeated.keys().next() {
                return Err(format!("{path:?} is not listed more than once"));
            }
            let named = current
                .snapshot
                .quarantined
                .iter()
                .any(|file| file.path == clear.path && file.object == clear.object);
            if !named {
                return Err(format!("{:?} is not set aside", clear.path));
            }
            let mut changes = strays_dropped(current);
            changes.insert(clear.path.clone(), None);
            Ok(changes)
        })
    }

    /// One write: judged against the head inside an immediate transaction,
    /// its changes made by `changes` from the board at the head (or a
    /// refusal), committed as a new revision of the current epoch.
    fn transact(
        &self,
        expected: &str,
        meta: &RevisionMeta,
        changes: impl FnOnce(&Current) -> Result<Changes, String>,
    ) -> Result<BoardWrite, BoardStoreError> {
        let mut writer = lock(&self.writer);
        let tx = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        // A retried write is answered as it was first.
        if let Some(write_id) = &meta.write_id {
            let done: Option<(u64, u64)> = tx
                .query_row(
                    "SELECT epoch, revision FROM revisions WHERE write_id = ?1",
                    [write_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(sql)?;
            if let Some((epoch, revision)) = done {
                return Ok(BoardWrite::Written(Head { epoch, revision }.board_head()));
            }
        }
        let head = head_in(&tx).map_err(sql)?.ok_or(BoardStoreError::NoBoard)?;
        let Some(asked) = Head::parse(expected) else {
            return Err(BoardStoreError::InvalidWrite(format!(
                "{expected:?} is not a board head"
            )));
        };
        // Whether the board ever stood at `asked`: its revision is on the
        // board, and its epoch was in force at that revision (it began at
        // or before it, and the next began at or after it).
        let epochs = epochs_in(&tx).map_err(sql)?;
        let held = asked.revision <= head.revision
            && epochs.iter().enumerate().any(|(at, (epoch, from, _))| {
                *epoch == asked.epoch
                    && *from <= asked.revision
                    && epochs
                        .get(at + 1)
                        .is_none_or(|(_, next, _)| *next >= asked.revision)
            });
        match (asked == head, held) {
            (true, _) => {}
            // A head this board held: the board moved on (or failed over)
            // since. Read it again.
            (false, true) => return Ok(BoardWrite::Lost),
            // An older epoch's head this board never held: a stale
            // primary's, fenced. Read it again.
            (false, false) if asked.epoch < head.epoch => return Ok(BoardWrite::Lost),
            (false, false) => {
                return Err(BoardStoreError::InvalidWrite(format!(
                    "{expected:?} is not a head of this board"
                )));
            }
        }
        if let Some(gate) = lock(&self.gate).clone() {
            gate().map_err(BoardStoreError::Unavailable)?;
        }
        let refusal: Option<String> = tx
            .query_row("SELECT v FROM meta WHERE k = 'refuse_next'", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(sql)?;
        if let Some(reason) = refusal {
            tx.execute("DELETE FROM meta WHERE k = 'refuse_next'", [])
                .map_err(sql)?;
            tx.commit().map_err(sql)?;
            return Ok(BoardWrite::Refused(reason));
        }
        // The writer lock is held, so the board read now is the head's.
        let current = self.current()?;
        if current.head != head {
            return Err(BoardStoreError::Unavailable(
                "the board moved under its writer".into(),
            ));
        }
        let mut changes = match changes(&current) {
            Ok(changes) => changes,
            Err(reason) => return Ok(BoardWrite::Refused(reason)),
        };
        for (path, bytes) in root_files() {
            changes.insert(path, Some((REGULAR.to_string(), bytes)));
        }
        let next = Head {
            epoch: head.epoch,
            revision: head.revision + 1,
        };
        commit_in(&tx, next, &changes, meta).map_err(sql)?;
        tx.commit().map_err(sql)?;
        drop(writer);
        self.committed(next.revision);
        Ok(BoardWrite::Written(next.board_head()))
    }

    /// Appends a revision of `changes` as they are, unjudged: an import,
    /// a follower applying its primary's revision, or a test spoiling a
    /// file. `at` is the revision it must be (the next), with its epoch.
    ///
    /// # Errors
    /// Not the next revision, or SQLite's.
    pub fn append_raw(
        &self,
        at: Head,
        changes: &[FileChange],
        meta: &RevisionMeta,
    ) -> Result<(), BoardStoreError> {
        let mut writer = lock(&self.writer);
        let tx = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let head = head_in(&tx).map_err(sql)?;
        let expected = head.map_or(1, |head| head.revision + 1);
        if at.revision != expected || head.is_some_and(|head| at.epoch < head.epoch) {
            return Err(BoardStoreError::InvalidWrite(format!(
                "revision {} of epoch {} is not the next (at {head:?})",
                at.revision, at.epoch
            )));
        }
        if head.map_or(true, |head| head.epoch != at.epoch) {
            tx.execute(
                "INSERT OR IGNORE INTO epochs(epoch, from_revision, reason, at) VALUES (?1, ?2, 'applied', ?3)",
                params![at.epoch, at.revision, now_text()],
            )
            .map_err(sql)?;
        }
        let changes: Changes = changes
            .iter()
            .map(|change| {
                (
                    change.path.clone(),
                    change
                        .bytes
                        .clone()
                        .map(|bytes| (change.mode.clone(), bytes)),
                )
            })
            .collect();
        commit_in(&tx, at, &changes, meta).map_err(sql)?;
        tx.commit().map_err(sql)?;
        drop(writer);
        self.committed(at.revision);
        Ok(())
    }

    /// Starts a new epoch at the head (a promotion or a restore): every
    /// head of an older epoch is fenced from now on. The epoch is recorded
    /// as the head's own, so the head moves to `epoch:revision` at once.
    ///
    /// # Errors
    /// No board, or SQLite's.
    pub fn bump_epoch(&self, reason: &str, at_least: u64) -> Result<Head, BoardStoreError> {
        let mut writer = lock(&self.writer);
        let tx = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let head = head_in(&tx).map_err(sql)?.ok_or(BoardStoreError::NoBoard)?;
        let epoch = (head.epoch + 1).max(at_least);
        tx.execute(
            "INSERT INTO epochs(epoch, from_revision, reason, at) VALUES (?1, ?2, ?3, ?4)",
            params![epoch, head.revision, reason, now_text()],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        drop(writer);
        *lock(&self.current) = None;
        let head = Head {
            epoch,
            revision: head.revision,
        };
        self.committed(head.revision);
        Ok(head)
    }

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

/// Every epoch, oldest first: its number, the revision it began at, why.
fn epochs_in(connection: &Connection) -> rusqlite::Result<Vec<(u64, u64, String)>> {
    let mut statement = connection
        .prepare_cached("SELECT epoch, from_revision, reason FROM epochs ORDER BY epoch")?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RevisionRow> {
    Ok(RevisionRow {
        revision: row.get(0)?,
        epoch: row.get(1)?,
        write_id: row.get(2)?,
        writer_name: row.get(3)?,
        writer_email: row.get(4)?,
        principal: row.get(5)?,
        message: row.get(6)?,
        kind: row.get(7)?,
        server_time: row.get(8)?,
        git_commit: row.get(9)?,
        git_meta: row.get(10)?,
    })
}

/// Every stray (a file set aside that no write carries) removed: what a
/// write drops, as the git store's does.
fn strays_dropped(current: &Current) -> Changes {
    current
        .snapshot
        .quarantined
        .iter()
        .filter(|file| !file.carried)
        .map(|file| (file.path.clone(), None))
        .collect()
}

/// The head: the latest revision, with the latest epoch.
fn head_in(connection: &Connection) -> rusqlite::Result<Option<Head>> {
    let revision: Option<u64> =
        connection.query_row("SELECT MAX(revision) FROM revisions", [], |row| row.get(0))?;
    let epoch: Option<u64> =
        connection.query_row("SELECT MAX(epoch) FROM epochs", [], |row| row.get(0))?;
    Ok(revision.map(|revision| Head {
        epoch: epoch.unwrap_or(1),
        revision,
    }))
}

fn live_files(connection: &Connection) -> rusqlite::Result<Vec<BoardFile>> {
    let mut statement = connection.prepare_cached(
        "SELECT path, mode, bytes FROM files WHERE removed IS NULL ORDER BY path",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(BoardFile {
                path: row.get(0)?,
                mode: row.get(1)?,
                bytes: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Commits `changes` as revision `at`: a put whose bytes and mode equal
/// the live file's changes nothing; every other put or removal closes the
/// live row.
fn commit_in(
    connection: &Connection,
    at: Head,
    changes: &Changes,
    meta: &RevisionMeta,
) -> rusqlite::Result<()> {
    for (path, change) in changes {
        let live: Option<(String, Vec<u8>)> = connection
            .query_row(
                "SELECT mode, bytes FROM files WHERE path = ?1 AND removed IS NULL",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match (change, &live) {
            (Some(put), Some(live)) if put == live => continue,
            (None, None) => continue,
            _ => {}
        }
        if live.is_some() {
            connection.execute(
                "UPDATE files SET removed = ?1 WHERE path = ?2 AND removed IS NULL",
                params![at.revision, path],
            )?;
        }
        if let Some((mode, bytes)) = change {
            connection.execute(
                "INSERT INTO files(path, mode, bytes, added) VALUES (?1, ?2, ?3, ?4)",
                params![path, mode, bytes, at.revision],
            )?;
        }
    }
    connection.execute(
        "INSERT INTO revisions(revision, epoch, write_id, writer_name, writer_email, principal, message, kind, \
         server_time, git_commit, git_meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            at.revision,
            at.epoch,
            meta.write_id,
            meta.writer.name,
            meta.writer.email,
            meta.principal,
            meta.message,
            meta.kind,
            meta.server_time.clone().unwrap_or_else(now_text),
            meta.git_commit,
            meta.git_meta
        ],
    )?;
    Ok(())
}
