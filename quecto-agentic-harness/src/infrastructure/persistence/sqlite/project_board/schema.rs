//! The board file's schema (epic #2705 B7, B11): this build writes 5 and
//! reads 4 or 5, upgrading 4 in place; and each revision's chain.
use super::storage::digest;
use super::*;
const VERSION: i64 = 5;
const APPLICATION: i64 = 0x51424f32;
/// Schema 4's revisions table: a writable open upgrades it, adding each
/// revision's chain (B11), and so to schema 5.
const REVISIONS_4: &str = "CREATE TABLE revisions (head TEXT PRIMARY KEY NOT NULL, root TEXT NOT NULL, name TEXT NOT NULL, email TEXT NOT NULL, revision INTEGER NOT NULL UNIQUE, parent INTEGER, message TEXT NOT NULL, at INTEGER NOT NULL, git_commit TEXT, git_meta BLOB)";
const ADD_CHAIN: &str = "ALTER TABLE revisions ADD COLUMN chain TEXT";
/// Every table a board holds: import requires each empty (#2741).
pub(super) const TABLES: [(&str, &str); 9] = [
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
        "CREATE TABLE revisions (head TEXT PRIMARY KEY NOT NULL, root TEXT NOT NULL, name TEXT NOT NULL, email TEXT NOT NULL, revision INTEGER NOT NULL UNIQUE, parent INTEGER, message TEXT NOT NULL, at INTEGER NOT NULL, git_commit TEXT, git_meta BLOB, chain TEXT)",
    ),
    (
        "state",
        "CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), head TEXT NOT NULL)",
    ),
    (
        "git_objects",
        "CREATE TABLE git_objects (oid TEXT PRIMARY KEY NOT NULL, raw_tree BLOB)",
    ),
    (
        "history_meta",
        "CREATE TABLE history_meta (id INTEGER PRIMARY KEY CHECK(id=1), board_id TEXT, algorithm TEXT NOT NULL, source_remote TEXT, source_ref TEXT, source_head TEXT, imported_count INTEGER NOT NULL)",
    ),
    (
        "epochs",
        "CREATE TABLE epochs (epoch INTEGER PRIMARY KEY, from_revision INTEGER NOT NULL, reason TEXT NOT NULL, at INTEGER NOT NULL)",
    ),
    // B7: who a board server let in for each revision it wrote.
    (
        "revision_principals",
        "CREATE TABLE revision_principals (head TEXT PRIMARY KEY NOT NULL, principal TEXT NOT NULL)",
    ),
    // B7: each served write's answer, by principal and write id, for retries.
    (
        "op_results",
        "CREATE TABLE op_results (principal TEXT NOT NULL, write_id TEXT NOT NULL, request TEXT NOT NULL, answer TEXT NOT NULL, PRIMARY KEY(principal, write_id))",
    ),
];
const INDEXES: [(&str, &str); 7] = [
    ("sqlite_autoindex_revisions_2", "revisions"),
    ("sqlite_autoindex_git_objects_1", "git_objects"),
    ("sqlite_autoindex_objects_1", "objects"),
    ("sqlite_autoindex_tree_entries_1", "tree_entries"),
    ("sqlite_autoindex_revisions_1", "revisions"),
    (
        "sqlite_autoindex_revision_principals_1",
        "revision_principals",
    ),
    ("sqlite_autoindex_op_results_1", "op_results"),
];

/// What a board file holds, as recognised: nothing yet, or a board of
/// schema 4 (the build before B11's) or 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Schema {
    Empty,
    Four,
    Five,
}

/// Recognises the board file `c` opens for writing, at `path`: an empty
/// one is made a schema-5 board, and a schema-4 one is upgraded in place,
/// behind a copy of it beside it, all under the write lock (the schema is
/// read again once the lock is held, so concurrent opens upgrade once).
pub(super) fn recognize(c: &mut Connection, path: &Path) -> Result<(), BoardStoreError> {
    let seen = {
        let tx = c.transaction().map_err(unavailable)?;
        let seen = schema_of(&tx)?;
        tx.commit().map_err(unavailable)?;
        seen
    };
    if seen == Schema::Five {
        return Ok(());
    }
    // Another open may hold the lock while it upgrades: wait for it.
    c.busy_timeout(UPGRADE_WAIT).map_err(unavailable)?;
    let recognized = recognize_locked(c, path);
    c.busy_timeout(OPEN_WAIT).map_err(unavailable)?;
    recognized
}

/// How long an open waits for the board's lock.
pub(super) const OPEN_WAIT: Duration = Duration::from_secs(5);

/// How long an open waits for the lock while another may be upgrading the
/// board (a copy, then a chain for each of up to 4,096 revisions).
const UPGRADE_WAIT: Duration = Duration::from_secs(120);

/// [`recognize`], once the write lock is held.
fn recognize_locked(c: &mut Connection, path: &Path) -> Result<(), BoardStoreError> {
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(unavailable)?;
    match schema_of(&tx)? {
        Schema::Five => {}
        Schema::Empty => {
            for (_, sql) in TABLES {
                tx.execute_batch(sql).map_err(unavailable)?;
            }
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(unavailable)?;
            tx.pragma_update(None, "application_id", APPLICATION)
                .map_err(unavailable)?;
        }
        Schema::Four => return super::schema_upgrade::upgrade_in_place(tx, path),
    }
    tx.commit().map_err(unavailable)
}

/// Recognises a board file read and never written (export, backup): a
/// board of schema 4 or 5.
pub(super) fn recognize_on(c: &Connection) -> Result<(), BoardStoreError> {
    match schema_of(c)? {
        Schema::Four | Schema::Five => Ok(()),
        Schema::Empty => Err(BoardStoreError::Clash(
            "not a board: the file holds no board yet".into(),
        )),
    }
}

/// The schema of the file `tx` reads, read-only: empty, 4 or 5, else
/// refused, saying what to do.
pub(super) fn schema_of(tx: &Connection) -> Result<Schema, BoardStoreError> {
    let version: i64 = tx
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(unavailable)?;
    let app: i64 = tx
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(unavailable)?;
    let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(CAST(name AS BLOB))+length(CAST(type AS BLOB))+length(CAST(tbl_name AS BLOB))+coalesce(length(CAST(sql AS BLOB)),0)),0) FROM sqlite_schema",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(unavailable)?;
    let unsupported = || match (app == APPLICATION, version > VERSION) {
        (true, true) => BoardStoreError::Clash(format!(
            "this board was written by a newer quecto (schema version {version}; this build \
             reads 4 and {VERSION}): upgrade this binary to read it"
        )),
        (_, _) => BoardStoreError::Clash(format!(
            "not a board this build reads (schema version {version}, this build writes \
             {VERSION}): move the file aside to start a new board"
        )),
    };
    if (count == 0 || count == (TABLES.len() + INDEXES.len()) as i64) && bytes <= 65536 {
    } else {
        return Err(unsupported());
    }
    let schema: Vec<(String, String, String, Option<String>)> = tx
        .prepare("SELECT name,type,tbl_name,sql FROM sqlite_schema ORDER BY name")
        .map_err(unavailable)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(unavailable)?
        .collect::<Result<_, _>>()
        .map_err(unavailable)?;
    let empty = version == 0 && app == 0 && schema.is_empty();
    let sql_of = |name: &str| TABLES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s);
    let current = sql_of("revisions").expect("the schema has a revisions table");
    // Whether this is the schema of `wanted`, whose revisions table is `revisions`.
    let of = |wanted: i64, revisions: &str| {
        let sql_of = |name: &str| match name {
            "revisions" => Some(revisions),
            other => sql_of(other),
        };
        version == wanted
            && app == APPLICATION
            && schema.len() == TABLES.len() + INDEXES.len()
            && schema
                .iter()
                .all(|(name, kind, table, sql)| match kind.as_str() {
                    "table" => table == name && sql.is_some() && sql.as_deref() == sql_of(name),
                    "index" => {
                        sql.is_none() && INDEXES.iter().any(|(n, t)| name == n && table == t)
                    }
                    _ => false,
                })
    };
    match (empty, of(VERSION, current), of(4, REVISIONS_4)) {
        (true, false, false) => Ok(Schema::Empty),
        (false, true, false) => Ok(Schema::Five),
        (false, false, true) => Ok(Schema::Four),
        _ => Err(unsupported()),
    }
}

/// Schema 4 to 5 (B11): each revision's chain, made in revision order from
/// what the board holds, so the same history gives the same chains on
/// every machine.
pub(super) fn upgrade(tx: &Connection) -> Result<(), BoardStoreError> {
    revisions_gapless(tx)?;
    let last: i64 = tx
        .query_row(
            "SELECT coalesce(max(revision), 0) FROM revisions",
            [],
            |r| r.get(0),
        )
        .map_err(unavailable)?;
    tx.execute_batch(ADD_CHAIN).map_err(unavailable)?;
    for revision in 1..=last {
        let chain = chain_of(tx, revision)?;
        tx.execute(
            "UPDATE revisions SET chain=?1 WHERE revision=?2",
            params![chain, revision],
        )
        .map_err(unavailable)?;
    }
    tx.pragma_update(None, "user_version", VERSION)
        .map_err(unavailable)
}

/// `Ok` while the board's revisions are numbered 1 to their count, with no
/// gap: only then can each be given its chain.
pub(super) fn revisions_gapless(c: &Connection) -> Result<(), BoardStoreError> {
    let (count, last): (i64, Option<i64>) = c
        .query_row("SELECT count(*), max(revision) FROM revisions", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(unavailable)?;
    match count == last.unwrap_or(0) {
        true => Ok(()),
        false => Err(unavailable(format!(
            "its revisions are not numbered 1 to {} with no gap ({count} are held)",
            last.unwrap_or(0)
        ))),
    }
}

/// Revision `revision`'s chain (B11): a SHA-256 over the chain before it
/// and every column of the revision and its principal, so two boards hold
/// one chain at a revision only when they hold one history to it. The
/// chain before it must be made.
pub(super) fn chain_of(c: &Connection, revision: i64) -> Result<String, BoardStoreError> {
    type Row = (
        String,
        String,
        String,
        String,
        Option<i64>,
        String,
        i64,
        Option<String>,
        Option<Vec<u8>>,
        Option<String>,
    );
    let row: Row = c
        .query_row(
            "SELECT r.head,r.root,r.name,r.email,r.parent,r.message,r.at,r.git_commit,r.git_meta,\
             p.principal FROM revisions r LEFT JOIN revision_principals p ON p.head=r.head \
             WHERE r.revision=?1",
            [revision],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .map_err(unavailable)?;
    let (head, root, name, email, parent, message, at, git_commit, git_meta, principal) = row;
    let before: Option<String> = match revision {
        1 => None,
        _ => {
            let held: Option<Option<String>> = c
                .query_row(
                    "SELECT chain FROM revisions WHERE revision=?1",
                    [revision - 1],
                    |r| r.get(0),
                )
                .optional()
                .map_err(unavailable)?;
            let missing = || unavailable(format!("revision {} has no chain", revision - 1));
            Some(held.flatten().ok_or_else(missing)?)
        }
    };
    // An absent value and an empty one differ.
    let maybe = |value: Option<&[u8]>| match value {
        None => vec![0u8],
        Some(bytes) => [&[1u8][..], bytes].concat(),
    };
    let parent = parent.map(i64::to_be_bytes);
    let fields = [
        maybe(before.as_deref().map(str::as_bytes)),
        head.into_bytes(),
        root.into_bytes(),
        name.into_bytes(),
        email.into_bytes(),
        revision.to_be_bytes().to_vec(),
        maybe(parent.as_ref().map(|bytes| &bytes[..])),
        message.into_bytes(),
        at.to_be_bytes().to_vec(),
        maybe(git_commit.as_deref().map(str::as_bytes)),
        maybe(git_meta.as_deref()),
        maybe(principal.as_deref().map(str::as_bytes)),
    ];
    let fields: Vec<&[u8]> = fields.iter().map(Vec::as_slice).collect();
    Ok(digest(b"chain", &fields))
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
