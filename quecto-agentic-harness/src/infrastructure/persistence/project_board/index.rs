//! The local index of a project's board. It is a SQLite cache of the board
//! branch at one commit, kept at `<git-common-dir>/quecto/project-board.sqlite`,
//! and it answers a listing's queries without reading every task file.
//!
//! It is only a cache. It records the board commit it was built from and
//! is rebuilt from the store's read whenever the board moves. It is never
//! moved back to an older commit of the same history. Its listings mean
//! what the capability's `QueryBoard` listings mean, and a parity test
//! holds them to that. Every listing carries the commit it was read at and
//! the files set aside. SQL only narrows the rows: each decoded task is
//! checked again in Rust.
//!
//! [`index_file`](super::index_file) keeps it on disk. Each rebuild is a
//! fresh file renamed over the old one, so a reader sees one whole board,
//! and a crash mid-rebuild leaves the last whole index in place.
use super::index_file::{self, Build};
use super::store::GitBoardStore;
use super::task_record::{decode, encode};
use crate::application::project_board::dto::BoardListing;
use crate::application::project_board::ports::{
    BoardHead, BoardSnapshot, BoardStoreError, ProjectBoardStore, QuarantinedFile,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::is_live;
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
    /// not one: refresh it.
    NeedsRebuild,
    /// The index holds something that does not read back: discard it and
    /// refresh.
    Corrupt(String),
    Sqlite(String),
    Io(String),
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

    /// The index of `store`'s repository.
    ///
    /// # Errors
    /// git cannot name the repository's common directory.
    pub fn of(store: &GitBoardStore) -> Result<Self, IndexError> {
        Ok(Self::at(store.index_path().map_err(IndexError::Store)?))
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
            Ok(Some(_) | None) | Err(IndexError::NeedsRebuild | IndexError::Corrupt(_)) => {
                match store.read() {
                    Ok(snapshot) => self.rebuild(&snapshot, revisions),
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
            Ok(Some(stored)) if self.behind(&head, &stored, revisions) => {
                Ok(Refreshed::Newer { head: stored })
            }
            Ok(Some(_) | None) | Err(IndexError::NeedsRebuild | IndexError::Corrupt(_)) => {
                self.publish(build, &dir, head)
            }
            Err(error) => Err(error),
        }
    }

    /// `head` is an older commit of `stored`'s history and the remote board
    /// has not been reset to it. An unknown ancestry is not behind: the
    /// snapshot is published and the next refresh corrects it.
    fn behind(&self, head: &str, stored: &str, revisions: &dyn BoardRevisions) -> bool {
        revisions.is_ancestor(head, stored).unwrap_or(false)
            && revisions.remote_head().ok().flatten().as_deref() != Some(head)
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
        self.listing(&query, TaskStatus::Ready.as_str(), |task, statuses| {
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
        self.listing(query, "1", |task, _| {
            let fields = task.fields();
            fields.status.may_hold_claim()
                && fields
                    .claim
                    .as_ref()
                    .is_some_and(|claim| !is_live(claim, now))
        })
    }

    /// # Errors
    /// The index cannot be read.
    pub fn by_status(&self, status: TaskStatus) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks WHERE status = ?1 ORDER BY id";
        self.listing(query, status.as_str(), |task, _| {
            task.fields().status == status
        })
    }

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// The index cannot be read.
    pub fn children(&self, parent: &Slug) -> Result<BoardListing, IndexError> {
        let query = "SELECT id, record FROM tasks WHERE parent = ?1 ORDER BY id";
        self.listing(query, parent.as_str(), |task, _| {
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
        self.listing(query, task.as_str(), |candidate, _| {
            candidate.fields().depends_on.contains(task)
        })
    }

    /// The tasks `query` narrows to with `value` and `wanted` keeps, the
    /// files set aside and the head, from one read of the index. Each row
    /// must decode to the task of its id.
    fn listing(
        &self,
        query: &str,
        value: &str,
        wanted: impl Fn(&Task, &BTreeMap<String, TaskStatus>) -> bool,
    ) -> Result<BoardListing, IndexError> {
        let (head, rows, statuses, quarantined) = self.read(|transaction| {
            let mut statement = transaction.prepare(query)?;
            let rows: Vec<(String, Vec<u8>)> = statement
                .query_map([value], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut statement = transaction.prepare("SELECT id, status FROM tasks")?;
            let statuses: Vec<(String, String)> = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            Ok((
                stored_head(transaction)?,
                rows,
                statuses,
                quarantined(transaction)?,
            ))
        })?;
        let corrupt = |what: String| IndexError::Corrupt(what);
        let head = head
            .filter(|head| full_commit_id(head))
            .ok_or_else(|| corrupt("no board commit".into()))?;
        let statuses = statuses
            .into_iter()
            .map(|(id, status)| match TaskStatus::parse(&status) {
                Some(status) => Ok((id, status)),
                None => Err(corrupt(format!("status {status:?} of {id:?}"))),
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut tasks = vec![];
        for (id, record) in rows {
            let task = decode(&record).map_err(|error| corrupt(error.to_string()))?;
            if task.fields().id.as_str() != id {
                return Err(corrupt(format!("row {id:?} holds another task")));
            }
            if wanted(&task, &statuses) {
                tasks.push(task);
            }
        }
        let quarantined = quarantined
            .into_iter()
            .map(|(path, id, reason)| {
                let id = id.map(|id| Slug::parse(&id)).transpose();
                let id = id.map_err(|error| corrupt(error.to_string()))?;
                Ok(QuarantinedFile { path, id, reason })
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
            Ok(false) => Err(IndexError::NeedsRebuild),
            Err(error) if index_file::stale(&error) => Err(IndexError::NeedsRebuild),
            Err(error) => Err(error.into()),
        };
        transaction.finish()?;
        connection
            .close()
            .map_err(|(_, error)| IndexError::from(error))?;
        outcome
    }
}

/// A full SHA-1 or SHA-256 commit id in lowercase hex.
fn full_commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn rows(snapshot: &BoardSnapshot) -> Result<Vec<Row>, IndexError> {
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
                record: encode(task).map_err(|error| IndexError::Corrupt(error.to_string()))?,
                depends_on: fields
                    .depends_on
                    .iter()
                    .map(|id| id.as_str().into())
                    .collect(),
            })
        })
        .collect()
}

/// The files set aside, by path: path, id and reason.
fn quarantined(
    transaction: &Transaction<'_>,
) -> rusqlite::Result<Vec<(String, Option<String>, String)>> {
    let mut statement =
        transaction.prepare("SELECT path, id, reason FROM quarantined ORDER BY path")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
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

fn fill(
    transaction: &Transaction<'_>,
    head: &str,
    rows: &[Row],
    quarantined: &[QuarantinedFile],
) -> Result<(), IndexError> {
    for row in rows {
        transaction.execute(
            "INSERT INTO tasks (id, status, parent, claimed, record) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.id, row.status, row.parent, row.claimed, row.record],
        )?;
        #[cfg(test)]
        if std::env::var_os(ABORT_IN_FILL).is_some() {
            std::process::abort();
        }
        for dependency in &row.depends_on {
            transaction.execute(
                "INSERT INTO dependencies (task, depends_on) VALUES (?1, ?2)",
                params![row.id, dependency],
            )?;
        }
    }
    for file in quarantined {
        transaction.execute(
            "INSERT INTO quarantined (path, id, reason) VALUES (?1, ?2, ?3)",
            params![file.path, file.id.as_ref().map(Slug::as_str), file.reason],
        )?;
    }
    transaction.execute("INSERT INTO meta (key, value) VALUES ('head', ?1)", [head])?;
    Ok(())
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
