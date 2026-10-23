//! Durable flat canonical files, serialized by SQLite IMMEDIATE transactions.
use crate::application::project_board::ports::*;
use crate::domain::project_board::{entities::claim::Identity, value_objects::schema_error::line};
use crate::infrastructure::persistence::project_board::{
    artifacts::file_of,
    board_tree::{Board, LIMITS, MAX_BOARD_ENTRIES, folder_file, from_rows, object_id},
    store::cleared,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::Mutex, time::Duration};

type Rows = Vec<(String, String, Vec<u8>)>;
pub struct SqliteProjectBoardStore {
    connection: Mutex<Connection>,
    writer: Identity,
}
fn unavailable(e: impl std::fmt::Display) -> BoardStoreError {
    BoardStoreError::Unavailable(e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> BoardStoreError {
    BoardStoreError::InvalidWrite(e.to_string())
}
fn rows(c: &Connection) -> Result<Rows, BoardStoreError> {
    c.prepare("SELECT path, object, bytes FROM files ORDER BY path")
        .map_err(unavailable)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)
}
fn head(c: &Connection) -> Result<BoardHead, BoardStoreError> {
    c.query_row("SELECT head FROM state WHERE id=1", [], |r| r.get(0))
        .optional()
        .map_err(unavailable)?
        .map(BoardHead)
        .ok_or(BoardStoreError::NoBoard)
}
fn put(c: &Connection, path: &str, bytes: &[u8]) -> Result<(), BoardStoreError> {
    let object = format!("{:x}", Sha256::digest(bytes));
    c.execute("INSERT INTO files VALUES (?1,?2,?3) ON CONFLICT(path) DO UPDATE SET object=excluded.object, bytes=excluded.bytes", params![path,object,bytes]).map_err(unavailable)?;
    Ok(())
}
fn carried(board: &Board) -> BTreeSet<String> {
    let tasks = board
        .tasks
        .keys()
        .map(|id| format!("tasks/{}.md", id.as_str()))
        .chain(board.kept.keys().map(|name| format!("tasks/{name}")));
    tasks
        .chain(
            board
                .folders
                .iter()
                .flat_map(|(folder, t)| t.files.keys().map(move |name| format!("{folder}/{name}"))),
        )
        .collect()
}
fn encode(artifacts: &[BoardArtifact]) -> Result<Vec<(String, Vec<u8>)>, BoardStoreError> {
    let mut encoded = vec![];
    for artifact in artifacts {
        let bytes = file_of(artifact).map_err(invalid)?;
        let path = match artifact {
            BoardArtifact::Task(task) => {
                format!("tasks/{}.md", task.fields().id.as_str())
            }
            _ => {
                let f = folder_file(&artifact.key(), bytes.clone()).map_err(invalid)?;
                format!("{}/{}", f.folder, f.name)
            }
        };
        encoded.push((path, bytes));
    }
    Ok(encoded)
}
impl SqliteProjectBoardStore {
    pub fn open(path: &Path, writer: Identity) -> Result<Self, BoardStoreError> {
        writer.validate("writer").map_err(invalid)?;
        let connection = Connection::open(path).map_err(unavailable)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(unavailable)?;
        match version {
            0 | 1 => {}
            _ => {
                return Err(BoardStoreError::Clash(
                    "unsupported SQLite board schema".into(),
                ));
            }
        }

        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(unavailable)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            PRAGMA user_version=1; CREATE TABLE IF NOT EXISTS revisions (head TEXT PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS state (id INTEGER PRIMARY KEY CHECK(id=1), head TEXT NOT NULL, refuse INTEGER NOT NULL DEFAULT 0, fault INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, object TEXT NOT NULL, bytes BLOB NOT NULL);").map_err(unavailable)?;
        Ok(Self {
            connection: Mutex::new(connection),
            writer,
        })
    }
    fn advance(&self, c: &Connection) -> Result<BoardHead, BoardStoreError> {
        let next = BoardHead(format!(
            "{:x}",
            Sha256::digest(uuid::Uuid::new_v4().as_bytes())
        ));
        c.execute(
            "INSERT INTO revisions VALUES (?1,?2,?3)",
            params![next.0, self.writer.name, self.writer.email],
        )
        .map_err(unavailable)?;
        c.execute("INSERT INTO state(id,head) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET head=excluded.head", [&next.0]).map_err(unavailable)?;
        Ok(next)
    }
    fn change(
        &self,
        expected: &BoardHead,
        artifacts: Option<&[BoardArtifact]>,
        clear: Option<&SetAsideClear>,
    ) -> Result<BoardWrite, BoardStoreError> {
        object_id(&expected.0).map_err(|_| invalid("not a board head"))?;
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        let current = head(&tx)?;
        let known: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM revisions WHERE head=?1)",
                [&expected.0],
                |r| r.get(0),
            )
            .map_err(unavailable)?;
        match (known, current == *expected) {
            (true, true) => {}
            (true, false) => return Ok(BoardWrite::Lost),
            (false, _) => return Err(invalid("not a board head")),
        }
        let (refuse, fault): (bool, bool) = tx
            .query_row("SELECT refuse,fault FROM state WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(unavailable)?;
        // Consume fixture flags durably even when the following transaction rolls back.
        tx.execute("UPDATE state SET refuse=0,fault=0", [])
            .map_err(unavailable)?;
        tx.commit().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        match head(&tx)? == *expected {
            true => {}
            false => return Ok(BoardWrite::Lost),
        }
        if refuse {
            return Ok(BoardWrite::Refused("fixture refusal".into()));
        }
        let before_rows = rows(&tx)?;
        let before = from_rows(&before_rows)?;
        let mut encoded = vec![];
        let after = match (artifacts, clear) {
            (Some(artifacts), None) => {
                let keys: BTreeSet<_> = artifacts.iter().map(BoardArtifact::key).collect();
                encoded = encode(artifacts)?;
                if let Some(file) = before
                    .quarantined
                    .iter()
                    .find(|f| f.carried && f.key.as_ref().is_some_and(|k| keys.contains(k)))
                {
                    return Ok(BoardWrite::Refused(format!(
                        "{} is set aside: {}",
                        file.path, file.reason
                    )));
                }

                before.clone()
            }
            (None, Some(clear)) => match cleared(&before, clear) {
                Ok((after, keeps)) => {
                    after.within_clear_bounds(
                        &before,
                        &after.folders_after(&[]),
                        LIMITS,
                        &keeps,
                    )?;
                    after
                }
                Err(why) => return Ok(BoardWrite::Refused(why)),
            },
            _ => unreachable!("one mutation kind"),
        };
        let keep = carried(&after);
        for (path, _, _) in &before_rows {
            if keep.contains(path) {
                continue;
            }
            tx.execute("DELETE FROM files WHERE path=?1", [path])
                .map_err(unavailable)?;
        }
        for (index, (path, bytes)) in encoded.iter().enumerate() {
            put(&tx, path, bytes)?;
            if fault && index == 0 {
                tx.execute_batch("INSERT INTO b2_injected_missing_table VALUES (1)")
                    .map_err(unavailable)?;
                unreachable!("injected SQL must fail");
            }
        }
        let after = from_rows(&rows(&tx)?).map_err(|e| match e {
            BoardStoreError::TooLarge(why) => BoardStoreError::WriteTooLarge(why),
            other => other,
        })?;
        if artifacts.is_some() {
            after.within_bounds(&[], &after.folders_after(&[]), LIMITS)?;
        }
        let next = self.advance(&tx)?;
        tx.commit().map_err(unavailable)?;
        Ok(BoardWrite::Written(next))
    }
    #[cfg(feature = "test-support")]
    pub fn connection_settings(&self) -> Result<(String, i64), BoardStoreError> {
        let c = self.connection.lock().map_err(unavailable)?;
        Ok((
            c.query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .map_err(unavailable)?,
            c.query_row("PRAGMA synchronous", [], |r| r.get(0))
                .map_err(unavailable)?,
        ))
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_put_raw(&self, path: &str, bytes: &[u8]) -> Result<(), BoardStoreError> {
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        head(&tx)?;
        put(&tx, path, bytes)?;
        self.advance(&tx)?;
        tx.commit().map_err(unavailable)
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_refuse_next_write(&self) -> Result<(), BoardStoreError> {
        self.connection
            .lock()
            .map_err(unavailable)?
            .execute("UPDATE state SET refuse=1", [])
            .map_err(unavailable)?;
        Ok(())
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_fail_after_first_row(&self) -> Result<(), BoardStoreError> {
        self.connection
            .lock()
            .map_err(unavailable)?
            .execute("UPDATE state SET fault=1", [])
            .map_err(unavailable)?;
        Ok(())
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_committed_by(&self, head: &BoardHead) -> Result<Identity, BoardStoreError> {
        self.connection
            .lock()
            .map_err(unavailable)?
            .query_row(
                "SELECT name,email FROM revisions WHERE head=?1",
                [&head.0],
                |r| {
                    Ok(Identity {
                        name: r.get(0)?,
                        email: r.get(1)?,
                    })
                },
            )
            .map_err(unavailable)
    }
}
impl ProjectBoardStore for SqliteProjectBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c.transaction().map_err(unavailable)?;
        let head = head(&tx)?;
        Ok(from_rows(&rows(&tx)?)?.snapshot(head))
    }
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        match head(&tx) {
            Ok(h) => Ok(BoardBootstrap::Existing(h)),
            Err(BoardStoreError::NoBoard) => {
                let h = self.advance(&tx)?;
                tx.commit().map_err(unavailable)?;
                Ok(BoardBootstrap::Created(h))
            }
            Err(e) => Err(e),
        }
    }
    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        line("message", message, 200).map_err(invalid)?;
        let keys: BTreeSet<_> = artifacts.iter().map(BoardArtifact::key).collect();
        match (artifacts.is_empty(), keys.len() == artifacts.len()) {
            (false, true) => self.change(expected, Some(artifacts), None),
            _ => Err(invalid("write one or more board files, each once")),
        }
    }
    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        line("message", message, 200).map_err(invalid)?;
        let copies: usize = clear
            .repeated
            .values()
            .map(|k| match k {
                KeepCopy::Object { seen, .. } | KeepCopy::Drop(seen) => seen.len(),
            })
            .sum();
        match (
            clear.repeated.len() <= MAX_BOARD_ENTRIES / 2,
            copies <= MAX_BOARD_ENTRIES,
        ) {
            (true, true) => {}
            _ => return Err(invalid("clear exceeds copy bounds")),
        }
        object_id(&clear.object).map_err(|_| invalid("not an object id"))?;
        for keep in clear.repeated.values() {
            let (kept, seen) = match keep {
                KeepCopy::Object { keep, seen } => (Some(keep), seen),
                KeepCopy::Drop(seen) => (None, seen),
            };
            for oid in kept.into_iter().chain(seen) {
                object_id(oid).map_err(|_| invalid("not an object id"))?;
            }
        }
        self.change(expected, None, Some(clear))
    }
    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}

#[cfg(all(test, feature = "test-support"))]
mod project_board_sqlite_tests;
