//! Immutable structural objects; no artifact or folder interpretation.
pub(super) use super::schema::{TABLES, chain_of, recognize, recognize_on};
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::{Entry, raw_entry_bytes};

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

/// The answer kept for `write` (B7); `Some(None)` when kept for another.
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
    c.busy_timeout(super::schema::OPEN_WAIT)
        .map_err(unavailable)?;
    recognize(&mut c, path)?;
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

/// The root tree of the revision `rev` names (#2705 B9): `<epoch>:<n>`,
/// canonical, with revision `n` within that epoch's interval, from its
/// first revision to the next epoch's first (an opened epoch's held
/// revision lies in both) or the newest; `None` for any other text.
pub(super) fn root_at(c: &Connection, rev: &BoardHead) -> Result<Option<String>, BoardStoreError> {
    let Ok((epoch, revision)) = super::history::parse_head(&rev.0) else {
        return Ok(None);
    };
    let root: Option<String> = c
        .query_row(
            "SELECT r.root FROM revisions r JOIN epochs e ON e.epoch=?1 \
             WHERE r.revision=?2 AND ?2>=e.from_revision AND ?2<=coalesce(\
             (SELECT from_revision FROM epochs WHERE epoch=?1+1),\
             (SELECT max(revision) FROM revisions))",
            params![epoch, revision],
            |r| r.get(0),
        )
        .optional()
        .map_err(unavailable)?;
    root.map(|root| object_id(&root)).transpose()
}

pub(super) fn current(c: &Connection) -> Result<(BoardHead, String), BoardStoreError> {
    let lengths:Option<(i64,Option<i64>)>=c.query_row("SELECT length(CAST(state.head AS BLOB)),length(CAST(revisions.root AS BLOB)) FROM state LEFT JOIN revisions ON revisions.head=state.head WHERE state.id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(unavailable)?;
    if lengths.is_none_or(|(head, root)| head <= 64 && root.is_some_and(|n| n <= 64)) {
    } else {
        return Err(unavailable("invalid state/revision shape"));
    }
    let row:Option<(String,Option<String>)>=c.query_row("SELECT state.head,revisions.root FROM state LEFT JOIN revisions ON revisions.head=state.head WHERE state.id=1",[],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(unavailable)?;
    match row {
        Some((head, Some(root))) => {
            let revision: i64 = c
                .query_row(
                    "SELECT revision FROM revisions WHERE head=?1",
                    [head],
                    |r| r.get(0),
                )
                .map_err(unavailable)?;
            let epoch: i64 = c
                .query_row("SELECT max(epoch) FROM epochs", [], |r| r.get(0))
                .map_err(unavailable)?;
            let (tip,start):(i64,i64)=c.query_row("SELECT (SELECT max(revision) FROM revisions),from_revision FROM epochs WHERE epoch=?1",[epoch],|r|Ok((r.get(0)?,r.get(1)?))).map_err(unavailable)?;
            if epoch > 0 && start > 0 && start <= revision && revision == tip {
            } else {
                return Err(unavailable("invalid current frontier"));
            }
            Ok((BoardHead(format!("{epoch}:{revision}")), object_id(&root)?))
        }
        Some((_, None)) => Err(unavailable("board state names a missing revision")),
        None => Err(BoardStoreError::NoBoard),
    }
}

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
pub(super) fn digest(kind: &[u8], fields: &[&[u8]]) -> String {
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

pub(super) fn tree_digest(rows: &[Entry]) -> Result<(String, i64), BoardStoreError> {
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
    Ok((oid, size))
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
    pub(super) fn insert(
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
        message: &str,
        principal: Option<&str>,
    ) -> Result<BoardHead, BoardStoreError> {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(unavailable)?
            .as_secs();
        let at = i64::try_from(at).map_err(unavailable)?;
        let previous: Option<i64> = self
            .connection
            .query_row("SELECT max(revision) FROM revisions", [], |r| r.get(0))
            .map_err(unavailable)?;
        let revision = previous
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| unavailable("revision overflow"))?;
        self.insert(
            "INSERT INTO epochs VALUES (1,1,'bootstrap',?1) ON CONFLICT(epoch) DO NOTHING",
            [at],
        )?;
        self.insert("INSERT INTO history_meta VALUES (1,NULL,'sha256',NULL,NULL,NULL,0) ON CONFLICT(id) DO NOTHING",[])?;
        let epoch: i64 = self
            .connection
            .query_row("SELECT max(epoch) FROM epochs", [], |r| r.get(0))
            .map_err(unavailable)?;
        let head = BoardHead(format!("{epoch}:{revision}"));
        self.insert(
            "INSERT INTO revisions VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,NULL,NULL)",
            params![
                head.0,
                root,
                writer.name,
                writer.email,
                revision,
                previous,
                message,
                at
            ],
        )?;
        if let Some(principal) = principal {
            self.insert(
                "INSERT INTO revision_principals VALUES (?1,?2)",
                params![head.0, principal],
            )?;
        }
        self.seal(revision)?;
        self.insert(
            "INSERT INTO state VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET head=excluded.head",
            [&head.0],
        )?;
        Ok(head)
    }
    /// Makes revision `revision`'s chain (B11), its row and principal
    /// written: the chain.
    pub(super) fn seal(&mut self, revision: i64) -> Result<String, BoardStoreError> {
        let chain = chain_of(self.connection, revision)?;
        self.insert(
            "UPDATE revisions SET chain=?1 WHERE revision=?2",
            params![chain, revision],
        )?;
        Ok(chain)
    }
    pub(super) fn blob(&mut self, bytes: &[u8]) -> Result<String, BoardStoreError> {
        let oid = blob_id(bytes);
        let held:Option<(String,Option<i64>,bool)>=self.connection.query_row("SELECT kind,length(bytes),EXISTS(SELECT 1 FROM git_objects WHERE git_objects.oid=objects.oid) FROM objects WHERE oid=?1",[&oid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(unavailable)?;
        if let Some((kind, len, imported)) = held {
            if matches!((kind.as_str(),len,imported),("blob",Some(n),false) if n==bytes.len() as i64)
            {
            } else {
                return Err(unavailable("object collision"));
            }
            let held: Vec<u8> = self
                .connection
                .query_row("SELECT bytes FROM objects WHERE oid=?1", [&oid], |r| {
                    r.get(0)
                })
                .map_err(unavailable)?;
            if held == bytes {
            } else {
                return Err(unavailable("object collision"));
            }
        }
        self.insert(
            "INSERT INTO objects VALUES (?1,'blob',?2,NULL) ON CONFLICT(oid) DO NOTHING",
            params![oid, bytes],
        )?;
        Ok(oid)
    }
    pub(super) fn tree(&mut self, rows: &[Entry]) -> Result<String, BoardStoreError> {
        let (oid, size) = tree_digest(rows)?;
        let held: Option<(String, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT kind,tree_size FROM objects WHERE oid=?1",
                [&oid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(unavailable)?;
        if let Some((kind, held_size)) = held {
            use crate::infrastructure::persistence::project_board::board_tree::Reader;
            let imported: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM git_objects WHERE oid=?1)",
                    [&oid],
                    |r| r.get(0),
                )
                .map_err(unavailable)?;
            if matches!((kind.as_str(),held_size,imported),("tree",Some(n),false) if n==size) {
            } else {
                return Err(unavailable("tree object collision"));
            }
            let stored = super::reader::structural(self.connection)
                .listing(&oid, 4 * 655360)
                .map_err(|_| unavailable("stored collision rows"))?;
            if stored.len() == rows.len()
                && stored
                    .iter()
                    .zip(rows)
                    .all(|(a, b)| a.mode == b.mode && a.raw == b.raw && a.oid == b.oid)
            {
            } else {
                return Err(unavailable("tree object collision"));
            }
        }
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
    /// Original objects use the same physical writer/collision seam as native
    /// rows; raw membership and the complete ordered projection are evidence.
    pub(super) fn imported(
        &mut self,
        o: &crate::application::project_board::ports::history::HistoryObject,
    ) -> Result<(), BoardStoreError> {
        use crate::application::project_board::ports::history::HistoryKind;
        use crate::infrastructure::persistence::project_board::board_tree::Reader;
        let (kind, bytes, size, raw) = match o.kind {
            HistoryKind::Blob => ("blob", Some(o.bytes.as_slice()), None, None),
            HistoryKind::Tree => {
                let mut size = 0usize;
                for row in &o.entries {
                    size = size
                        .checked_add(raw_entry_bytes(&row.mode, &row.name, &row.oid))
                        .ok_or_else(|| unavailable("import tree overflow"))?;
                }
                (
                    "tree",
                    None,
                    Some(i64::try_from(size).map_err(unavailable)?),
                    Some(o.bytes.as_slice()),
                )
            }
            _ => return Err(unavailable("commit in object table")),
        };
        type Held = (String, Option<Vec<u8>>, Option<i64>, Option<Vec<u8>>);
        let held: Option<Held> = self.connection.query_row("SELECT kind,bytes,tree_size,(SELECT raw_tree FROM git_objects WHERE oid=objects.oid) FROM objects WHERE oid=?1", [&o.oid], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(unavailable)?;
        if let Some((k, b, n, r)) = held {
            let member: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM git_objects WHERE oid=?1)",
                    [&o.oid],
                    |r| r.get(0),
                )
                .map_err(unavailable)?;
            if member && k == kind && b.as_deref() == bytes && n == size && r.as_deref() == raw {
            } else {
                return Err(unavailable("import object collision"));
            }
            if kind == "tree" {
                let rows = super::reader::structural(self.connection)
                    .listing(&o.oid, 4 * 655360)
                    .map_err(|_| unavailable("import collision rows"))?;
                if rows.len() == o.entries.len()
                    && rows
                        .iter()
                        .zip(&o.entries)
                        .all(|(a, b)| a.mode == b.mode && a.raw == b.name && a.oid == b.oid)
                {
                } else {
                    return Err(unavailable("import tree collision"));
                }
            }
            return Ok(());
        }
        self.insert(
            "INSERT INTO objects VALUES (?1,?2,?3,?4)",
            params![o.oid, kind, bytes, size],
        )?;
        self.insert(
            "INSERT INTO git_objects VALUES (?1,?2)",
            params![o.oid, raw],
        )?;
        for (i, e) in o.entries.iter().enumerate() {
            self.insert(
                "INSERT INTO tree_entries VALUES (?1,?2,?3,?4,?5)",
                params![
                    o.oid,
                    i64::try_from(i).map_err(unavailable)?,
                    e.mode,
                    e.name,
                    e.oid
                ],
            )?;
        }
        Ok(())
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
#[cfg(feature = "test-support")]
pub(super) fn advance_message(
    c: &Connection,
    root: &str,
    writer: &Identity,
    message: &str,
) -> Result<BoardHead, BoardStoreError> {
    ObjectWriter::new(c).advance(root, writer, message, None)
}
pub(super) fn blob(c: &Connection, bytes: &[u8]) -> Result<String, BoardStoreError> {
    ObjectWriter::new(c).blob(bytes)
}
pub(super) fn tree(c: &Connection, rows: &[Entry]) -> Result<String, BoardStoreError> {
    ObjectWriter::new(c).tree(rows)
}

#[cfg(all(test, feature = "test-support"))]
pub(super) fn advance(
    c: &Connection,
    root: &str,
    writer: &Identity,
) -> Result<BoardHead, BoardStoreError> {
    advance_message(c, root, writer, "board: fixture edit")
}
