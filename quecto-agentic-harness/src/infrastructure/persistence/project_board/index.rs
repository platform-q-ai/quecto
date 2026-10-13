//! The local index of a project's board. It is a SQLite cache of the board
//! branch at one commit, kept at
//! `<git-common-dir>/quecto/project-board/<remote>.sqlite` (one per remote,
//! its name spelt by [`file_name`], in a directory of its own that only
//! this user may write), and it answers a listing's queries
//! without reading every task file. Anyone who can write the repository
//! can forge it, so its listings are advice: writes go through the store's
//! compare-and-swap.
//!
//! It is only a cache. It records the board commit it was built from and
//! is rebuilt from the store's read whenever the board moves. It is never
//! moved back to an older commit of the same history. Its listings mean
//! what the capability's `QueryBoard` listings mean, and a parity test
//! holds them to that. Every listing carries the commit it was read at and
//! the files set aside. Each task's record is its canonical markdown file
//! (the task codec's, #2481 S8). SQL only narrows the rows: each decoded
//! task is checked again in Rust.
//!
//! [`index_file`](super::index_file) keeps it on disk. Each rebuild is a
//! fresh file renamed over the old one, so a reader sees one whole board,
//! and a crash mid-rebuild leaves the last whole index in place.
use super::artifacts::task::TaskCodec;
use super::artifacts::{decode, encode};
use super::index_file::{self, Build};
use super::store::GitBoardStore;
use crate::application::project_board::dto::BoardListing;
use crate::application::project_board::ports::{
    ArtifactKey, BoardHead, BoardSnapshot, BoardStoreError, ProjectBoardStore, QuarantinedFile,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::is_live;
use crate::domain::project_board::value_objects::shown_text::{
    shown_board_path, shown_char, shown_path,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use rusqlite::{Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use index_file::{BUSY_TIMEOUT, INDEX_SCHEMA, MAX_VALUE_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    Store(BoardStoreError),
    /// No index, one of another layout or record schema, or a file that is
    /// not one: refresh it. Carries why (SQLite's own cause, when it gave
    /// one), so an error that outlives the rebuild says what it was.
    NeedsRebuild(String),
    /// The index holds something that does not read back: discard it and
    /// refresh.
    Corrupt(String),
    Sqlite(String),
    Io(String),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(BoardStoreError::NoBoard) => f.write_str("there is no board"),
            Self::Store(
                BoardStoreError::Clash(why)
                | BoardStoreError::InvalidWrite(why)
                | BoardStoreError::Unavailable(why),
            ) => f.write_str(why),
            Self::NeedsRebuild(why) => write!(f, "the index needs a rebuild: {why}"),
            Self::Corrupt(why) => write!(f, "the index is corrupt: {why}"),
            Self::Sqlite(why) => write!(f, "the index's SQLite failed: {why}"),
            Self::Io(why) => write!(f, "the index's files: {why}"),
        }
    }
}

/// What a refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refreshed {
    Rebuilt {
        head: String,
    },
    Unchanged {
        head: String,
    },
    /// The index already holds `head`, a later commit of the same history
    /// than the snapshot, which was read before the board moved on.
    Newer {
        head: String,
    },
}

/// What the index needs to know of the board's history beyond a snapshot.
pub trait BoardRevisions: Send + Sync {
    /// The remote board's commit, or `None` when there is no board.
    ///
    /// # Errors
    /// The remote cannot be reached.
    fn remote_head(&self) -> Result<Option<String>, BoardStoreError>;

    /// Whether `older` is `newer` or one of its ancestors.
    ///
    /// # Errors
    /// Either commit is unknown here.
    fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool, BoardStoreError>;
}

impl BoardRevisions for GitBoardStore {
    fn remote_head(&self) -> Result<Option<String>, BoardStoreError> {
        self.head()
    }

    fn is_ancestor(&self, older: &str, newer: &str) -> Result<bool, BoardStoreError> {
        GitBoardStore::is_ancestor(self, older, newer)
    }
}

/// One task as the index stores it, encoded before any lock is taken.
struct Row {
    id: String,
    status: &'static str,
    parent: Option<String>,
    claimed: bool,
    record: Vec<u8>,
    depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardIndex {
    path: PathBuf,
}

impl BoardIndex {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The index of `store`'s repository and remote: in its common git
    /// directory, shared by every worktree and never in a tree.
    ///
    /// # Errors
    /// git cannot name the repository's common directory.
    pub fn of(store: &GitBoardStore) -> Result<Self, IndexError> {
        let common = store.common_dir().map_err(IndexError::Store)?;
        let dir = common.join("quecto").join("project-board");
        Ok(Self::at(dir.join(file_name(store.remote()))))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || "index".into(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// Rebuilds from the board when the remote board has moved since the
    /// index was built, or the index cannot be read. A board that is gone
    /// takes the index with it.
    ///
    /// # Errors
    /// The store's, or the index file's.
    pub fn refresh(
        &self,
        store: &dyn ProjectBoardStore,
        revisions: &dyn BoardRevisions,
    ) -> Result<Refreshed, IndexError> {
        let Some(remote) = revisions.remote_head().map_err(IndexError::Store)? else {
            return self.gone();
        };
        match self.head() {
            Ok(Some(head)) if head == remote => Ok(Refreshed::Unchanged { head }),
            Ok(Some(_) | None) | Err(IndexError::NeedsRebuild(_) | IndexError::Corrupt(_)) => {
                match store.read() {
                    Ok(snapshot) => self.rebuild_at(&snapshot, revisions, Some(&remote)),
                    Err(BoardStoreError::NoBoard) => self.gone(),
                    Err(error) => Err(IndexError::Store(error)),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn gone(&self) -> Result<Refreshed, IndexError> {
        self.discard()?;
        Err(IndexError::Store(BoardStoreError::NoBoard))
    }

    /// Removes the index, so every read answers [`IndexError::NeedsRebuild`].
    ///
    /// # Errors
    /// The index's directory cannot be used.
    pub fn discard(&self) -> Result<(), IndexError> {
        let dir = index_file::directory(&self.path)?;
        let _lock = index_file::lock(&dir, &self.name())?;
        index_file::discard(&self.path)
    }

    /// Makes the index hold exactly `snapshot`, unless it already holds
    /// that commit, or a later one of the same history while the remote
    /// board is not back at the snapshot's (a reset still rebuilds).
    ///
    /// # Errors
    /// A snapshot head that is not a full commit id, or the index file's.
    pub fn rebuild(
        &self,
        snapshot: &BoardSnapshot,
        revisions: &dyn BoardRevisions,
    ) -> Result<Refreshed, IndexError> {
        // Asked before the writer lock is taken, never while it is held: an
        // unreachable remote is no remote, and keeps nothing.
        let remote = revisions.remote_head().ok().flatten();
        self.rebuild_at(snapshot, revisions, remote.as_deref())
    }

    /// [`Self::rebuild`] with the remote board's head already known.
    fn rebuild_at(
        &self,
        snapshot: &BoardSnapshot,
        revisions: &dyn BoardRevisions,
        remote: Option<&str>,
    ) -> Result<Refreshed, IndexError> {
        let head = snapshot.head.0.clone();
        if !full_commit_id(&head) {
            return Err(IndexError::Corrupt(format!(
                "{head:?} is not a board commit"
            )));
        }
        if self.head().ok().flatten().as_deref() == Some(head.as_str()) {
            return Ok(Refreshed::Unchanged { head });
        }
        let rows = rows(snapshot)?;
        let dir = index_file::directory(&self.path)?;
        let name = self.name();
        let build = index_file::build(&dir, &name, |transaction| {
            fill(transaction, &head, &rows, &snapshot.quarantined)
        })?;
        let _lock = index_file::lock(&dir, &name)?;
        index_file::sweep(&dir, &name);
        match self.head() {
            Ok(Some(stored)) if stored == head => Ok(Refreshed::Unchanged { head }),
            Ok(Some(stored)) if behind(&head, &stored, remote, revisions) => {
                Ok(Refreshed::Newer { head: stored })
            }
            Ok(Some(_) | None) | Err(IndexError::NeedsRebuild(_) | IndexError::Corrupt(_)) => {
                self.publish(build, &dir, head)
            }
            Err(error) => Err(error),
        }
    }

    fn publish(&self, build: Build, dir: &Path, head: String) -> Result<Refreshed, IndexError> {
        index_file::publish(build, &self.path, dir)?;
        Ok(Refreshed::Rebuilt { head })
    }

    /// The board commit the index was built from.
    ///
    /// # Errors
    /// [`IndexError::NeedsRebuild`] for no index or a foreign one.
    pub fn head(&self) -> Result<Option<String>, IndexError> {
        let stored = self.read(stored_head)?;
        match stored {
            Some(head) if full_commit_id(&head) => Ok(Some(head)),
            Some(head) => Err(IndexError::Corrupt(format!("stored head {head:?}"))),
            None => Ok(None),
        }
    }

    /// Ready tasks whose every dependency is on the board and releases its
    /// dependents ([`TaskStatus::releases_dependents`]): the ones anyone
    /// may start now.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn ready(&self) -> Result<BoardListing, IndexError> {
        // The statuses the domain's rule releases on, spelt from its own
        // closed vocabulary.
        let releasing = TaskStatus::ALL
            .iter()
            .filter(|status| status.releases_dependents())
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        assert!(!releasing.is_empty(), "some status releases its dependents");
        let query = format!(
            "SELECT id, record FROM tasks WHERE status = ?1 AND NOT EXISTS (\
                 SELECT 1 FROM dependencies LEFT JOIN tasks AS dependency \
                 ON dependency.id = dependencies.depends_on \
                 WHERE dependencies.task = tasks.id \
                 AND (dependency.status IS NULL OR dependency.status NOT IN ({releasing}))\
             ) ORDER BY id"
        );
        self.listing(&query, &[TaskStatus::Ready.as_str()], |task, statuses| {
            let fields = task.fields();
            let released = |id: &Slug| {
                statuses
                    .get(id.as_str())
                    .is_some_and(|status| status.releases_dependents())
            };
            fields.status == TaskStatus::Ready && fields.depends_on.iter().all(released)
        })
    }

    /// Held tasks whose claim expired by `now`: anyone may take them over.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn takeable(&self, now: &Timestamp) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks WHERE claimed = ?1 ORDER BY id";
        self.listing(query, &["1"], |task, _| {
            let fields = task.fields();
            fields.status.may_hold_claim()
                && fields
                    .claim
                    .as_ref()
                    .is_some_and(|claim| !is_live(claim, now))
        })
    }

    /// Every task on the board.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn all(&self) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks ORDER BY id";
        self.listing(query, &[], |_, _| true)
    }

    /// # Errors
    /// The index cannot be read.
    pub fn by_status(&self, status: TaskStatus) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks WHERE status = ?1 ORDER BY id";
        self.listing(query, &[status.as_str()], |task, _| {
            task.fields().status == status
        })
    }

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn children(&self, parent: &Slug) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks WHERE parent = ?1 ORDER BY id";
        self.listing(query, &[parent.as_str()], |task, _| {
            task.fields().parent.as_ref() == Some(parent)
        })
    }

    /// The tasks that depend on `task`.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn dependents(&self, task: &Slug) -> Result<BoardListing, IndexError> {
        let query = "SELECT tasks.id, record FROM tasks JOIN dependencies ON dependencies.task = tasks.id \
                     WHERE dependencies.depends_on = ?1 ORDER BY tasks.id";
        self.listing(query, &[task.as_str()], |candidate, _| {
            candidate.fields().depends_on.contains(task)
        })
    }

    /// The tasks `query` narrows to with `value` and `wanted` keeps, the
    /// files set aside and the head, from one read of the index. Each row
    /// must decode to the task of its id.
    fn listing(
        &self,
        query: &str,
        values: &[&str],
        wanted: impl Fn(&Task, &BTreeMap<String, TaskStatus>) -> bool,
    ) -> Result<BoardListing, IndexError> {
        let (head, rows, depended, quarantined) = self.read(|transaction| {
            let mut statement = transaction.prepare(query)?;
            let rows: Vec<(String, Vec<u8>)> = statement
                .query_map(rusqlite::params_from_iter(values), |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
            // Every task some task depends on, by its record: the status
            // column is only a prefilter.
            let mut statement = transaction.prepare(
                "SELECT id, record FROM tasks WHERE id IN (SELECT depends_on FROM dependencies)",
            )?;
            let depended: Vec<(String, Vec<u8>)> = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok((
                stored_head(transaction)?,
                rows,
                depended,
                quarantined(transaction)?,
            ))
        })?;
        let corrupt = |what: String| IndexError::Corrupt(what);
        let head = head
            .filter(|head| full_commit_id(head))
            .ok_or_else(|| corrupt("no board commit".into()))?;
        let statuses = depended
            .into_iter()
            .map(|(id, record)| {
                let task =
                    decode::<TaskCodec>(&record).map_err(|error| corrupt(error.to_string()))?;
                match task.fields().id.as_str() == id {
                    true => Ok((id, task.fields().status)),
                    false => Err(corrupt(format!("row {id:?} holds another task"))),
                }
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut tasks = vec![];
        for (id, record) in rows {
            let task = decode::<TaskCodec>(&record).map_err(|error| corrupt(error.to_string()))?;
            match task.fields().id.as_str() == id {
                true => {}
                false => return Err(corrupt(format!("row {id:?} holds another task"))),
            }
            if wanted(&task, &statuses) {
                tasks.push(task);
            }
        }
        let quarantined = quarantined
            .into_iter()
            .map(|(path, id, reason, object, carried)| {
                let id = id.map(|id| Slug::parse(&id)).transpose();
                let id = id.map_err(|error| corrupt(error.to_string()))?;
                match full_commit_id(&object) {
                    true => {}
                    false => return Err(corrupt(format!("object {object:?} of {path:?}"))),
                }
                // A path and reason as a store shows them, or the row was
                // planted: a task's file in `tasks/`, a stray in any folder
                // the store reads.
                let shown = match &id {
                    Some(_) => shown_path(&path),
                    None => shown_board_path(&path),
                };
                match shown {
                    true => {}
                    false => return Err(corrupt(format!("path {path:?} no store shows"))),
                }
                match reason.chars().all(|c| shown_char(c) || c == ' ') {
                    true => {}
                    false => return Err(corrupt(format!("reason of {path:?} no store gives"))),
                }
                Ok(QuarantinedFile {
                    path,
                    key: id.map(ArtifactKey::Task),
                    reason,
                    object,
                    carried,
                })
            })
            .collect::<Result<_, IndexError>>()?;
        Ok(BoardListing {
            head: BoardHead(head),
            tasks,
            quarantined,
        })
    }

    /// One consistent read of an index of this layout and record schema.
    fn read<T>(
        &self,
        body: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, IndexError> {
        let mut connection = index_file::reader(&self.path)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let outcome = match index_file::is_current(&transaction) {
            Ok(true) => body(&transaction).map_err(IndexError::from),
            Ok(false) => Err(IndexError::NeedsRebuild(
                "another layout or record schema".into(),
            )),
            Err(error) => Err(error.into()),
        };
        transaction.finish()?;
        connection
            .close()
            .map_err(|(_, error)| IndexError::from(error))?;
        outcome
    }
}

/// The stored head is kept over an older snapshot `head` only while the
/// remote board still holds it: `head` is one of its ancestors, and it is
/// the remote board's head or one of its ancestors. A head a force push
/// abandoned, an unknown ancestry or no remote board keeps nothing: the
/// snapshot is published and the next refresh corrects it.
fn behind(head: &str, stored: &str, remote: Option<&str>, revisions: &dyn BoardRevisions) -> bool {
    let ancestor = |older: &str, newer: &str| revisions.is_ancestor(older, newer).unwrap_or(false);
    ancestor(head, stored) && remote.is_some_and(|remote| ancestor(stored, remote))
}

/// The index's file name for `remote`: `<remote>.sqlite`, the name spelt
/// in `a-z`, `0-9`, `-` and `_`, every other byte percent-encoded, so no
/// two remotes share one even on a file system that ignores case. A name
/// spelt longer than [`MAX_SPELT`] keeps its first 32 bytes and a hash of
/// the whole, so a build file's name stays far under 255 bytes.
pub fn file_name(remote: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut spelt = String::new();
    for byte in remote.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' => spelt.push(char::from(byte)),
            _ => spelt.push_str(&format!("%{byte:02X}")),
        }
    }
    let spelt = match spelt.len() <= MAX_SPELT {
        true => spelt,
        false => {
            let digest = Sha256::digest(remote.as_bytes());
            let hash: String = digest[..8]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("{}~{hash}", &spelt[..32])
        }
    };
    assert!(
        spelt.is_ascii() && spelt.len() <= MAX_SPELT,
        "a short, plain name"
    );
    format!("{spelt}.sqlite")
}

/// The longest spelt remote name an index's file name holds.
const MAX_SPELT: usize = 64;

/// A full SHA-1 or SHA-256 commit id in lowercase hex.
fn full_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether the index can hold what `snapshot` sets aside: until it keeps
/// a table per kind (S12) its `quarantined` table names a task's file or a
/// stray only, so a file set aside under any other kind's key fails closed
/// rather than be listed as a stray. Other kinds the board holds are no
/// bar: their listings are read from the store's snapshot until S12, and
/// task listings go on.
fn set_aside_indexable(snapshot: &BoardSnapshot) -> Result<(), IndexError> {
    match snapshot
        .quarantined
        .iter()
        .find(|file| !task_or_stray(file))
    {
        None => Ok(()),
        Some(file) => Err(not_indexed(format!("{} is set aside", file.path))),
    }
}

fn not_indexed(what: String) -> IndexError {
    IndexError::Store(BoardStoreError::Unavailable(format!(
        "the board index names set-aside task files only for now, and {what}"
    )))
}

/// A file set aside as a task's, or as no artifact's: all the index's
/// `quarantined` table can name until S12.
fn task_or_stray(file: &QuarantinedFile) -> bool {
    matches!(file.key, None | Some(ArtifactKey::Task(_)))
}

fn rows(snapshot: &BoardSnapshot) -> Result<Vec<Row>, IndexError> {
    set_aside_indexable(snapshot)?;
    snapshot
        .tasks
        .iter()
        .map(|task| {
            let fields = task.fields();
            Ok(Row {
                id: fields.id.as_str().into(),
                status: fields.status.as_str(),
                parent: fields.parent.as_ref().map(|parent| parent.as_str().into()),
                claimed: fields.claim.is_some(),
                record: encode::<TaskCodec>(task)
                    .map_err(|error| IndexError::Corrupt(error.to_string()))?
                    .into_bytes(),
                depends_on: fields
                    .depends_on
                    .iter()
                    .map(|id| id.as_str().into())
                    .collect(),
            })
        })
        .collect()
}

/// One file set aside as the index holds it: path, id, reason, object and
/// whether every write carries it.
type SetAside = (String, Option<String>, String, String, bool);

fn quarantined(transaction: &Transaction<'_>) -> rusqlite::Result<Vec<SetAside>> {
    let mut statement = transaction.prepare(
        "SELECT path, id, reason, object, carried FROM quarantined ORDER BY path, rowid",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    rows.collect()
}

fn stored_head(transaction: &Transaction<'_>) -> rusqlite::Result<Option<String>> {
    let mut statement = transaction.prepare("SELECT value FROM meta WHERE key = 'head'")?;
    let mut rows = statement.query_map([], |row| row.get(0))?;
    rows.next().transpose()
}

/// Where a test process dies inside a rebuild, to prove a crash leaves
/// the last whole index.
#[cfg(test)]
pub const ABORT_IN_FILL: &str = "QUECTO_BOARD_INDEX_ABORT_IN_FILL";

/// Test-only: ends the process at once, as a crash does (no destructor
/// runs, so the build is left half made). A child its test runs with
/// `without_core_dumps` ends with `_exit(134)` (#2192's convention), so no
/// core dump is written for the crash the test stages; otherwise it aborts.
#[cfg(test)]
fn end_as_a_crash() -> ! {
    crate::infrastructure::test_support::no_core_dumps::fatal_end(std::process::abort)()
}

fn fill(
    transaction: &Transaction<'_>,
    head: &str,
    rows: &[Row],
    quarantined: &[QuarantinedFile],
) -> rusqlite::Result<()> {
    for row in rows {
        transaction.execute(
            "INSERT INTO tasks (id, status, parent, claimed, record) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.id, row.status, row.parent, row.claimed, row.record],
        )?;
        #[cfg(test)]
        if std::env::var_os(ABORT_IN_FILL).is_some() {
            end_as_a_crash();
        }
        for dependency in &row.depends_on {
            transaction.execute(
                "INSERT INTO dependencies (task, depends_on) VALUES (?1, ?2)",
                params![row.id, dependency],
            )?;
        }
    }
    for file in quarantined {
        assert!(task_or_stray(file), "rows refused {:?}", file.key);
        transaction.execute(
            "INSERT INTO quarantined (path, id, reason, object, carried) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file.path,
                file.id().map(Slug::as_str),
                file.reason,
                file.object,
                file.carried
            ],
        )?;
    }
    transaction.execute("INSERT INTO meta (key, value) VALUES ('head', ?1)", [head])?;
    Ok(())
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
