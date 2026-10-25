//! Contract for [`BoardOpStore`] (epic #2705 B7), on the SQLite adapter a
//! board server writes through: one operation runs once per write id, in
//! one transaction its every write lands in or none does, as the
//! committer it names, recorded beside the principal; reads see only what
//! has committed.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use quecto::application::project_board::ports::{
    BoardOpStore, BoardStoreError, OpWrite, ProjectBoardStore, Transacted, WriteId,
};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;

use super::project_board_store as store;

fn identity(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn write(id: &str) -> OpWrite {
    OpWrite {
        write_id: WriteId::parse(id).expect("a write id"),
        request: "cli\n{}".into(),
        committer: identity("Pocket"),
        principal: "token:phone".into(),
    }
}

struct Board {
    _dir: tempfile::TempDir,
    path: PathBuf,
    ops: SqliteBoardOps,
}

fn board() -> Board {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("board.sqlite3");
    let ops = SqliteBoardOps::open(&path, identity("Server")).expect("a SQLite board");
    let made = ops.transact(
        &write("00000000-0000-4000-8000-000000000000"),
        &mut |store| store.bootstrap().map(|_| "made".to_string()),
    );
    assert_eq!(made, Ok(Transacted::Answered("made".into())));
    Board {
        _dir: dir,
        path,
        ops,
    }
}

/// `(name, principal)` of each revision, oldest first.
fn revisions(path: &Path) -> Vec<(String, Option<String>)> {
    let db = rusqlite::Connection::open(path).expect("the board");
    let mut rows = db
        .prepare(
            "SELECT r.name, p.principal FROM revisions r \
             LEFT JOIN revision_principals p ON p.head = r.head ORDER BY r.rowid",
        )
        .expect("a query");
    rows.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("revisions")
}

/// Writes task `id` on `store`'s board as it stands.
fn written(store: &dyn ProjectBoardStore, id: &str) -> Result<String, BoardStoreError> {
    let head = store.read()?.head;
    store.write_tasks(&head, &[store::task(id, "A task")], "board: create")?;
    Ok(format!("wrote {id}"))
}

#[test]
fn a_write_runs_once_per_write_id() {
    let board = board();
    let first = board.ops.transact(
        &write("aaaaaaaa-0000-4000-8000-000000000000"),
        &mut |store| written(&*store, "a"),
    );
    assert_eq!(first, Ok(Transacted::Answered("wrote a".into())));
    let again = board
        .ops
        .transact(&write("aaaaaaaa-0000-4000-8000-000000000000"), &mut |_| {
            panic!("a kept write id runs nothing")
        });
    assert_eq!(again, Ok(Transacted::Replayed("wrote a".into())));
    assert_eq!(revisions(&board.path).len(), 2, "made, then written once");
}

/// Every write one operation makes lands together, as its committer,
/// beside its principal; a failed operation writes nothing and keeps no
/// answer, so its retry runs.
#[test]
fn an_operation_lands_whole_or_not_at_all() {
    let board = board();
    let failed = board.ops.transact(
        &write("bbbbbbbb-0000-4000-8000-000000000000"),
        &mut |store| {
            written(&*store, "a")?;
            Err(BoardStoreError::Unavailable("the op failed".into()))
        },
    );
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(revisions(&board.path).len(), 1, "undone whole");
    let retried = board.ops.transact(
        &write("bbbbbbbb-0000-4000-8000-000000000000"),
        &mut |store| {
            assert_eq!(store.writer(), identity("Pocket"));
            written(&*store, "a")?;
            written(&*store, "b")
        },
    );
    assert_eq!(retried, Ok(Transacted::Answered("wrote b".into())));
    let last = revisions(&board.path).pop().expect("a revision");
    assert_eq!(last, ("Pocket".into(), Some("token:phone".into())));
    let ids: Vec<String> = board
        .ops
        .reads()
        .read()
        .expect("read")
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

/// A read sees only what has committed: never an operation's writes
/// while it runs.
#[test]
fn reads_see_only_what_has_committed() {
    let board = board();
    let reads: Arc<dyn ProjectBoardStore> = board.ops.reads();
    let before = reads.read().expect("read").head;
    let during = board.ops.transact(
        &write("cccccccc-0000-4000-8000-000000000000"),
        &mut |store| {
            written(&*store, "a")?;
            Ok(reads.read()?.head.0)
        },
    );
    assert_eq!(during, Ok(Transacted::Answered(before.0.clone())));
    assert_ne!(reads.read().expect("read").head, before, "committed");
}

/// A write id is kept per principal, for one request and committer: the
/// same id for another is `Reused`, and another principal's is its own.
#[test]
fn a_write_id_is_kept_per_principal_and_request() {
    let board = board();
    let id = "dddddddd-0000-4000-8000-000000000000";
    let first = board
        .ops
        .transact(&write(id), &mut |store| written(&*store, "a"));
    assert_eq!(first, Ok(Transacted::Answered("wrote a".into())));
    let never = |_| -> Result<String, BoardStoreError> { panic!("a reused id runs nothing") };
    let other = OpWrite {
        request: "cli\n{\"op\":\"list\"}".into(),
        ..write(id)
    };
    assert_eq!(
        board.ops.transact(&other, &mut never.clone()),
        Ok(Transacted::Reused)
    );
    let by = OpWrite {
        committer: identity("Mallory"),
        ..write(id)
    };
    assert_eq!(
        board.ops.transact(&by, &mut never.clone()),
        Ok(Transacted::Reused)
    );
    let theirs = OpWrite {
        principal: "token:laptop".into(),
        ..write(id)
    };
    let own = board
        .ops
        .transact(&theirs, &mut |store| written(&*store, "b"));
    assert_eq!(own, Ok(Transacted::Answered("wrote b".into())));
    assert!(WriteId::parse("dddddddd").is_none(), "a UUID only");
    assert!(WriteId::parse("DDDDDDDD-0000-4000-8000-000000000000").is_none());
}

/// A board of another schema version is refused, naming both versions
/// and what to do.
#[test]
fn a_board_of_another_version_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("board.sqlite3");
    let old = rusqlite::Connection::open(&path).expect("a database");
    old.execute_batch("CREATE TABLE state (id INTEGER); PRAGMA user_version = 2;")
        .expect("an old board");
    drop(old);
    let Err(BoardStoreError::Clash(why)) = SqliteBoardOps::open(&path, identity("Server")) else {
        panic!("refused")
    };
    assert!(
        why.contains("schema version 2, this build writes 4"),
        "{why}"
    );
    assert!(why.contains("move the file aside"), "{why}");
}
