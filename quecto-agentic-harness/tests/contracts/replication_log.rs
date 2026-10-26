//! Contract for [`ReplicationLog`] (epic #2705 B11) on the SQLite board a
//! primary serves: a batch is the objects after a follower's, then the
//! revisions after its revision (within the limit, once every object is
//! sent), with the primary's chain at the revision it follows (none past
//! its tip: a follower ahead of it is a fork). Each revision's chain is
//! made as it is written, and a schema-4 board is given its chains as it
//! opens, the same every time.
use std::path::Path;
use std::sync::Arc;

use quecto::application::project_board::ports::BoardTokens;
use quecto::application::project_board::ports::feed::BatchLimit;
use quecto::application::project_board::ports::replication::{AdminRows, ReplicationLog};
use quecto::application::project_board::use_cases::ServeReplication;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::timestamp::Timestamp;
use quecto::infrastructure::persistence::file::board_tokens::FileBoardTokens;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;

use super::board_feed::{board, identity, wrote};

/// A server's state: its board and its tokens.
pub(super) struct Server {
    _dir: tempfile::TempDir,
    pub(super) ops: Arc<SqliteBoardOps>,
    pub(super) tokens: Arc<FileBoardTokens>,
    pub(super) file: std::path::PathBuf,
}

pub(super) fn server() -> Server {
    let (dir, ops) = board();
    let tokens = FileBoardTokens::open(&dir.path().join("state")).expect("tokens");
    Server {
        file: dir.path().join("board.sqlite3"),
        _dir: dir,
        ops: Arc::new(ops),
        tokens: Arc::new(tokens),
    }
}

/// Every replicated table of the board file at `path`, row by row in key
/// order, each value as SQLite holds it (an object's place in order too).
pub(super) fn dump(path: &Path) -> Vec<String> {
    let c = rusqlite::Connection::open(path).unwrap();
    let mut rows = Vec::new();
    for query in [
        "SELECT rowid, * FROM objects ORDER BY rowid",
        "SELECT * FROM tree_entries ORDER BY tree, ordinal",
        "SELECT * FROM git_objects ORDER BY oid",
        "SELECT * FROM revisions ORDER BY revision",
        "SELECT * FROM revision_principals ORDER BY head",
        "SELECT * FROM epochs ORDER BY epoch",
        "SELECT * FROM history_meta",
        "SELECT * FROM state",
    ] {
        let mut statement = c.prepare(query).unwrap();
        let columns = statement.column_count();
        let mut read = statement.query([]).unwrap();
        while let Some(row) = read.next().unwrap() {
            let values: Vec<String> = (0..columns)
                .map(|at| format!("{:?}", row.get_ref(at).unwrap()))
                .collect();
            rows.push(format!("{query}: {}", values.join(" | ")));
        }
    }
    rows
}

pub(super) fn chains(path: &Path) -> Vec<Option<String>> {
    let c = rusqlite::Connection::open(path).unwrap();
    let mut statement = c
        .prepare("SELECT chain FROM revisions ORDER BY revision")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn the_log_batches_within_its_limit_with_the_chain_it_follows() {
    let primary = server();
    wrote(&primary.ops, 1, true, 4);
    let limit = BatchLimit::of(2).unwrap();
    let first = primary.ops.replica_batch(0, 0, limit).unwrap();
    assert_eq!((first.after, first.chain_after.as_deref()), (0, None));
    assert!(!first.objects.is_empty());
    let numbers: Vec<u64> = first.revisions.iter().map(|r| r.revision).collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!((first.tip, first.more), (5, true));
    assert_eq!(first.epochs.len(), 1);
    let held = first.objects.last().unwrap().sequence;
    let next = primary.ops.replica_batch(2, held, limit).unwrap();
    assert_eq!(next.chain_after, Some(first.revisions[1].chain.clone()));
    assert!(
        next.objects.is_empty(),
        "objects already sent are not again"
    );
    let numbers: Vec<u64> = next.revisions.iter().map(|r| r.revision).collect();
    assert_eq!(numbers, [3, 4]);
    // Past the tip there is no chain: a follower ahead is a fork.
    let past = primary.ops.replica_batch(9, held, limit).unwrap();
    assert_eq!(
        (past.chain_after, past.revisions.len(), past.more),
        (None, 0, false)
    );
}

#[test]
fn a_batch_is_a_read_and_its_admin_rows_are_the_tokens_alone() {
    let primary = server();
    wrote(&primary.ops, 1, true, 2);
    let at = Timestamp::parse("2026-10-26T09:00:00Z").unwrap();
    let phone = primary
        .tokens
        .add(&Slug::parse("phone").unwrap(), &at)
        .unwrap();
    let before = dump(&primary.file);
    let logged = primary.ops.replica_batch(0, 0, BatchLimit::MOST).unwrap();
    assert!(logged.tokens.is_empty(), "the log holds no admin row");
    let served = ServeReplication::new(
        Arc::clone(&primary.ops) as Arc<dyn ReplicationLog>,
        Arc::clone(&primary.tokens) as Arc<dyn AdminRows>,
    );
    let batch = served.batch(0, 0, BatchLimit::MOST).unwrap();
    assert_eq!(batch.revisions, logged.revisions);
    let names: Vec<&str> = batch.tokens.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["phone"]);
    let shown = format!("{batch:?}");
    assert!(!shown.contains(&phone.secret), "never a token");
    assert!(dump(&primary.file) == before, "a batch writes nothing");
}

#[test]
fn a_schema_4_board_is_given_its_chain_as_it_opens_the_same_every_time() {
    let primary = server();
    wrote(&primary.ops, 1, true, 3);
    let made = chains(&primary.file);
    drop(primary.ops);
    {
        let c = rusqlite::Connection::open(&primary.file).unwrap();
        c.execute_batch("ALTER TABLE revisions DROP COLUMN chain; PRAGMA user_version=4;")
            .unwrap();
    }
    let reopened = SqliteBoardOps::open(&primary.file, identity()).expect("schema 4 upgraded");
    assert_eq!(chains(&primary.file), made);
    assert!(made.iter().all(Option::is_some));
    let version: i64 = rusqlite::Connection::open(&primary.file)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 5);
    drop(reopened);
}
