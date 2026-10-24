//! Immutable structural objects; no artifact or folder interpretation.
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{Entry, raw_entry_bytes};

const VERSION: i64 = 3;
const APPLICATION: i64 = 0x51424f32;
const TABLES: [(&str, &str); 6] = [
    (
        "objects",
        "CREATE TABLE objects (oid TEXT PRIMARY KEY NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('blob','tree')), bytes BLOB, tree_size INTEGER, CHECK((kind='blob' AND typeof(bytes)='blob' AND tree_size IS NULL) OR (kind='tree' AND bytes IS NULL AND typeof(tree_size)='integer' AND tree_size>=0)))",
    ),
    (
        "tree_entries",
        "CREATE TABLE tree_entries (tree TEXT NOT NULL, ordinal INTEGER NOT NULL CHECK(ordinal>=0), mode TEXT NOT NULL, raw_name BLOB NOT NULL CHECK(typeof(raw_name)='blob'), oid TEXT NOT NULL, PRIMARY KEY(tree,ordinal))",
    ),
    (
        "revisions",
        "CREATE TABLE revisions (head TEXT PRIMARY KEY NOT NULL, root TEXT NOT NULL, name TEXT NOT NULL, email TEXT NOT NULL)",
    ),
    (
        "state",
        "CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), head TEXT NOT NULL)",
    ),
    // B7: who a board server let in for each revision it wrote.
    (
        "revision_principals",
        "CREATE TABLE revision_principals (head TEXT PRIMARY KEY NOT NULL, principal TEXT NOT NULL)",
    ),
    // B7: each served write's answer, by its write id, for its retries.
    (
        "op_results",
        "CREATE TABLE op_results (principal TEXT NOT NULL, write_id TEXT NOT NULL, request TEXT NOT NULL, answer TEXT NOT NULL, PRIMARY KEY(principal, write_id))",
    ),
];
const INDEXES: [(&str, &str); 5] = [
    ("sqlite_autoindex_objects_1", "objects"),
    ("sqlite_autoindex_tree_entries_1", "tree_entries"),
    ("sqlite_autoindex_revisions_1", "revisions"),
    (
        "sqlite_autoindex_revision_principals_1",
        "revision_principals",
    ),
    ("sqlite_autoindex_op_results_1", "op_results"),
];

/// The served writes' answers kept for their retries (B7), newest first.
const KEPT_ANSWERS: i64 = 1024;

/// A store call's own transaction, or inside a served operation's (B7) a
/// savepoint of it, so a failed call undoes its own rows alone.
pub(super) enum Scope<'c> {
    Own(rusqlite::Transaction<'c>),
    Within(rusqlite::Savepoint<'c>),
}

impl Scope<'_> {
    /// Immediate when it `writes`.
    pub(super) fn begin(c: &mut Connection, writes: bool) -> Result<Scope<'_>, BoardStoreError> {
        let behavior = match writes {
            true => TransactionBehavior::Immediate,
            false => TransactionBehavior::Deferred,
        };
        match c.is_autocommit() {
            true => c.transaction_with_behavior(behavior).map(Scope::Own),
            false => c.savepoint().map(Scope::Within),
        }
        .map_err(unavailable)
    }

    pub(super) fn commit(self) -> Result<(), BoardStoreError> {
        match self {
            Self::Own(tx) => tx.commit(),
            Self::Within(savepoint) => savepoint.commit(),
        }
        .map_err(unavailable)
    }
}

impl std::ops::Deref for Scope<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match self {
            Self::Own(tx) => tx,
            Self::Within(savepoint) => savepoint,
        }
    }
}

/// The answer kept for `write`'s id and principal, if any: `Ok(None)`
/// when that id was kept for another request or committer (B7).
pub(super) fn kept_answer(
    c: &Connection,
    write: &OpWrite,
) -> Result<Option<Option<String>>, BoardStoreError> {
    let kept: Option<(String, String)> = c
        .query_row(
            "SELECT request, answer FROM op_results WHERE principal=?1 AND write_id=?2",
            [&write.principal, write.write_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(unavailable)?;
    Ok(kept.map(|(request, answer)| (request == asked(write)).then_some(answer)))
}

/// Keeps `answer` for `write`, dropping the oldest past [`KEPT_ANSWERS`].
pub(super) fn keep_answer(
    c: &Connection,
    write: &OpWrite,
    answer: &str,
) -> Result<(), BoardStoreError> {
    c.execute(
        "INSERT INTO op_results VALUES (?1, ?2, ?3, ?4)",
        params![
            write.principal,
            write.write_id.as_str(),
            asked(write),
            answer
        ],
    )
    .map_err(unavailable)?;
    c.execute(
        "DELETE FROM op_results WHERE rowid <= (SELECT max(rowid) FROM op_results) - ?1",
        [KEPT_ANSWERS],
    )
    .map_err(unavailable)?;
    Ok(())
}

/// What `write` asks: its request and committer, hashed.
fn asked(write: &OpWrite) -> String {
    let OpWrite {
        request, committer, ..
    } = write;
    let fields = [request, &committer.name, &committer.email].map(|field| field.as_bytes());
    digest(b"op", &fields)
}

pub(super) fn open(path: &Path) -> Result<Connection, BoardStoreError> {
    let mut c = Connection::open(path).map_err(unavailable)?;
    c.busy_timeout(Duration::from_secs(5))
        .map_err(unavailable)?;
    // Recognition is entirely read-only, including before journal-mode changes.
    let tx = c.transaction().map_err(unavailable)?;
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(unavailable)?;
    let app: i64 = tx
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(unavailable)?;
    let schema: Vec<(String, String, String, Option<String>)> = tx
        .prepare("SELECT name,type,tbl_name,sql FROM sqlite_schema ORDER BY name")
        .map_err(unavailable)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)?;
    let empty = version == 0 && app == 0 && schema.is_empty();
    let recognized = version == VERSION
        && app == APPLICATION
        && schema.len() == TABLES.len() + INDEXES.len()
        && schema
            .iter()
            .all(|(name, kind, table, sql)| match kind.as_str() {
                "table" => {
                    table == name
                        && TABLES
                            .iter()
                            .any(|(n, s)| name == n && sql.as_deref() == Some(*s))
                }
                "index" => sql.is_none() && INDEXES.iter().any(|(n, t)| name == n && table == t),
                _ => false,
            });
    match (empty, recognized) {
        (true, false) => {
            for (_, sql) in TABLES {
                tx.execute_batch(sql).map_err(unavailable)?;
            }
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(unavailable)?;
            tx.pragma_update(None, "application_id", APPLICATION)
                .map_err(unavailable)?;
        }
        (false, true) => {}
        _ => {
            return Err(BoardStoreError::Clash(format!(
                "{} is not a board this build reads (schema version {version}, this build \
                 writes {VERSION}): move it aside to start a new board",
                path.display()
            )));
        }
    }
    tx.commit().map_err(unavailable)?;
    // SQLite may successfully execute the request but retain another journal
    // mode (notably for transient databases). Expose only durable WAL stores.
    let journal: String = c
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .map_err(unavailable)?;
    match journal.as_str() {
        "wal" => {}
        _ => return Err(unavailable("SQLite board requires WAL journal mode")),
    }
    c.pragma_update(None, "synchronous", "FULL")
        .map_err(unavailable)?;
    let synchronous: i64 = c
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .map_err(unavailable)?;
    match synchronous {
        2 => Ok(c),
        _ => Err(unavailable("SQLite board requires FULL synchronous mode")),
    }
}

pub(super) fn current(c: &Connection) -> Result<(BoardHead, String), BoardStoreError> {
    let row:Option<(String,Option<String>)>=c.query_row("SELECT state.head,revisions.root FROM state LEFT JOIN revisions ON revisions.head=state.head WHERE state.id=1",[],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(unavailable)?;
    match row {
        Some((head, Some(root))) => Ok((BoardHead(object_id(&head)?), object_id(&root)?)),
        Some((_, None)) => Err(unavailable("board state names a missing revision")),
        None => Err(BoardStoreError::NoBoard),
    }
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
fn digest(kind: &[u8], fields: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    frame(&mut hash, b"quecto-board-object-v2");
    frame(&mut hash, kind);
    for field in fields {
        frame(&mut hash, field);
    }
    format!("{:x}", hash.finalize())
}
pub(super) fn blob_id(bytes: &[u8]) -> String {
    digest(b"blob", &[bytes])
}
pub(super) fn entry(mode: &str, raw: &[u8], oid: String) -> Entry {
    Entry {
        mode: mode.into(),
        oid,
        name: String::from_utf8_lossy(raw).into_owned(),
        raw: raw.to_vec(),
    }
}

/// All physical mutation rows pass one seam. Ordinary builds carry only the
/// caller's transaction connection; pending fault state is feature-only.
pub(super) struct ObjectWriter<'a> {
    connection: &'a Connection,
    #[cfg(feature = "test-support")]
    fault: Option<super::fixture::Fault>,
}
impl<'a> ObjectWriter<'a> {
    pub(super) fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            #[cfg(feature = "test-support")]
            fault: None,
        }
    }
    #[cfg(feature = "test-support")]
    pub(super) fn with_fault(mut self, fault: Option<super::fixture::Fault>) -> Self {
        self.fault = fault;
        self
    }
    fn insert(
        &mut self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<usize, BoardStoreError> {
        let changed = self.connection.execute(sql, params).map_err(unavailable)?;
        #[cfg(feature = "test-support")]
        if changed > 0 && matches!(self.fault, Some(super::fixture::Fault::AfterFirstRow)) {
            self.fault = None;
            // Guaranteed SQL syntax error, immediately after the first actual
            // changed row (not a deduplicated object insertion attempt).
            self.connection
                .execute_batch("INSERT INTO")
                .map_err(unavailable)?;
            unreachable!("injected SQL syntax must fail");
        }
        Ok(changed)
    }
    pub(super) fn advance(
        &mut self,
        root: &str,
        writer: &Identity,
        principal: Option<&str>,
    ) -> Result<BoardHead, BoardStoreError> {
        let head = BoardHead(digest(
            b"revision",
            &[root.as_bytes(), uuid::Uuid::new_v4().as_bytes()],
        ));
        self.insert(
            "INSERT INTO revisions VALUES (?1,?2,?3,?4)",
            params![head.0, root, writer.name, writer.email],
        )?;
        if let Some(principal) = principal {
            self.insert(
                "INSERT INTO revision_principals VALUES (?1,?2)",
                params![head.0, principal],
            )?;
        }
        self.insert(
            "INSERT INTO state VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET head=excluded.head",
            [&head.0],
        )?;
        Ok(head)
    }
    pub(super) fn blob(&mut self, bytes: &[u8]) -> Result<String, BoardStoreError> {
        let oid = blob_id(bytes);
        self.insert(
            "INSERT INTO objects VALUES (?1,'blob',?2,NULL) ON CONFLICT(oid) DO NOTHING",
            params![oid, bytes],
        )?;
        Ok(oid)
    }
    pub(super) fn tree(&mut self, rows: &[Entry]) -> Result<String, BoardStoreError> {
        let mut hash = Sha256::new();
        frame(&mut hash, b"quecto-board-object-v2");
        frame(&mut hash, b"tree");
        frame(&mut hash, &(rows.len() as u64).to_be_bytes());
        let mut size = 0usize;
        for row in rows {
            object_id(&row.oid)?;
            size = size
                .checked_add(raw_entry_bytes(&row.mode, &row.raw, &row.oid))
                .ok_or_else(|| unavailable("tree size overflow"))?;
            for field in [row.mode.as_bytes(), row.raw.as_slice(), row.oid.as_bytes()] {
                frame(&mut hash, field);
            }
        }
        let size = i64::try_from(size).map_err(unavailable)?;
        let oid = format!("{:x}", hash.finalize());
        let inserted = self.insert(
            "INSERT INTO objects VALUES (?1,'tree',NULL,?2) ON CONFLICT(oid) DO NOTHING",
            params![oid, size],
        )?;
        if inserted == 1 {
            for (ordinal, row) in rows.iter().enumerate() {
                self.insert(
                    "INSERT INTO tree_entries VALUES (?1,?2,?3,?4,?5)",
                    params![oid, ordinal as i64, row.mode, row.raw, row.oid],
                )?;
            }
        }
        Ok(oid)
    }
    pub(super) fn mapped_tree(
        &mut self,
        files: &BTreeMap<String, String>,
    ) -> Result<String, BoardStoreError> {
        crate::infrastructure::persistence::project_board::board_tree::within(files, LIMITS)?;
        let rows = files
            .iter()
            .map(|(name, meta)| {
                let fields: Vec<_> = meta.split(' ').collect();
                match fields.as_slice() {
                    [mode, _, oid] => Ok(entry(mode, name.as_bytes(), object_id(oid)?)),
                    _ => Err(unavailable("invalid carried entry")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.tree(&rows)
    }
}
pub(super) fn advance(
    c: &Connection,
    root: &str,
    writer: &Identity,
    principal: Option<&str>,
) -> Result<BoardHead, BoardStoreError> {
    ObjectWriter::new(c).advance(root, writer, principal)
}
pub(super) fn blob(c: &Connection, bytes: &[u8]) -> Result<String, BoardStoreError> {
    ObjectWriter::new(c).blob(bytes)
}
pub(super) fn tree(c: &Connection, rows: &[Entry]) -> Result<String, BoardStoreError> {
    ObjectWriter::new(c).tree(rows)
}
