//! The schema-5 upgrade (epic #2705 B11 review), on the SQLite board a
//! server opens: a schema-4 board is given its chains as it opens, the
//! same every time, behind a whole copy of it beside it (and refused,
//! left as it was, if that copy cannot be made); concurrent opens upgrade
//! it once; a board of a newer build says to upgrade this binary.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use quecto::application::project_board::ports::BoardStoreError;
use quecto::infrastructure::persistence::sqlite::project_board::SqliteBoardOps;
use rusqlite::Connection;

use super::board_feed::{board, identity, wrote};
use super::replication_log::chains;

/// A schema-4 board of 4 revisions (as the build before B11's wrote it):
/// its temp dir, its file and the chains its revisions had at schema 5.
fn schema_4_board() -> (tempfile::TempDir, PathBuf, Vec<Option<String>>) {
    let (dir, ops) = board();
    wrote(&ops, 1, true, 3);
    drop(ops);
    let file = dir.path().join("board.sqlite3");
    let made = chains(&file);
    let c = Connection::open(&file).unwrap();
    c.execute_batch("ALTER TABLE revisions DROP COLUMN chain; PRAGMA user_version=4;")
        .unwrap();
    (dir, file, made)
}

fn version(file: &Path) -> i64 {
    let c = Connection::open(file).unwrap();
    c.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// The copies of `file` made before an upgrade.
fn copies(file: &Path) -> Vec<PathBuf> {
    let stem = format!("{}.schema4-", file.file_name().unwrap().to_str().unwrap());
    let mut found: Vec<PathBuf> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&stem)
        })
        .collect();
    found.sort();
    found
}

#[test]
fn a_schema_4_board_is_given_its_chain_behind_a_whole_copy() {
    let (_dir, file, made) = schema_4_board();
    let reopened = SqliteBoardOps::open(&file, identity()).expect("schema 4 upgraded");
    assert_eq!(
        chains(&file),
        made,
        "the same chains as written at schema 5"
    );
    assert!(made.iter().all(Option::is_some));
    assert_eq!(version(&file), 5);
    let kept = copies(&file);
    assert_eq!(kept.len(), 1, "one copy: {kept:?}");
    let copy = &kept[0];
    let name = copy.file_name().unwrap().to_str().unwrap();
    let stamp = name.strip_prefix("board.sqlite3.schema4-").unwrap();
    let shaped = stamp.len() == 16
        && stamp.char_indices().all(|(at, c)| match at {
            8 => c == 'T',
            15 => c == 'Z',
            _ => c.is_ascii_digit(),
        });
    assert!(shaped, "named as 20261026T105649Z: {name}");
    assert_eq!(version(copy), 4, "the copy is the board before the upgrade");
    let mode = std::fs::metadata(copy).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let rows = |path: &Path| -> i64 {
        let c = Connection::open(path).unwrap();
        c.query_row("SELECT count(*) FROM revisions", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(rows(copy), 4);
    let journal: String = Connection::open(copy)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "delete", "the copy is one file of its own");
    drop(reopened);
    // Opened again, it is schema 5: no second copy.
    drop(SqliteBoardOps::open(&file, identity()).unwrap());
    assert_eq!(copies(&file).len(), 1);
}

#[test]
fn a_copy_that_cannot_be_made_refuses_the_upgrade_and_leaves_the_board() {
    let (dir, file, _) = schema_4_board();
    // Out of WAL first, so opening needs no new file beside it: only the
    // copy cannot be made in a directory no one may write.
    let c = Connection::open(&file).unwrap();
    let journal: String = c
        .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    drop(c);
    let read_only = std::fs::Permissions::from_mode(0o500);
    std::fs::set_permissions(dir.path(), read_only).unwrap();
    let refused = SqliteBoardOps::open(&file, identity());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let Err(BoardStoreError::Unavailable(why)) = refused else {
        panic!("the upgrade is refused: {:?}", refused.err());
    };
    assert!(why.contains("was not upgraded to schema 5"), "{why}");
    assert!(why.contains("its copy could not be made"), "{why}");
    assert_eq!(version(&file), 4, "the board is left as it was");
    assert!(copies(&file).is_empty());
}

/// B11 review 2: an upgrade that fails after its copy leaves the board as
/// it was and removes the copy, so a server restarted again and again never
/// fills the disk; the refusal says what was refused and why.
#[test]
fn an_upgrade_that_fails_after_its_copy_removes_it_and_says_why() {
    let (_dir, file, _) = schema_4_board();
    // A board whose revisions have a gap cannot be given its chains.
    let c = Connection::open(&file).unwrap();
    c.execute_batch("DELETE FROM revisions WHERE revision=2")
        .unwrap();
    drop(c);
    for _ in 0..3 {
        let Err(BoardStoreError::Unavailable(why)) = SqliteBoardOps::open(&file, identity()) else {
            panic!("the upgrade is refused");
        };
        assert!(why.contains("was not upgraded to schema 5"), "{why}");
        assert!(why.contains("no gap"), "{why}");
        assert_eq!(version(&file), 4, "left as it was");
        assert!(
            copies(&file).is_empty(),
            "no copy is left: {:?}",
            copies(&file)
        );
    }
}

/// B11 review 2: a whole copy already beside the board (from an attempt
/// cut short) is used again, never a second one made.
#[test]
fn a_whole_copy_already_beside_the_board_is_used_again() {
    let (_dir, file, made) = schema_4_board();
    let c = Connection::open(&file).unwrap();
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    drop(c);
    let earlier = file.with_file_name("board.sqlite3.schema4-20200101T000000Z");
    std::fs::copy(&file, &earlier).unwrap();
    let journal: String = Connection::open(&earlier)
        .unwrap()
        .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
    drop(SqliteBoardOps::open(&file, identity()).unwrap());
    assert_eq!(chains(&file), made);
    assert_eq!(copies(&file), vec![earlier], "no second copy");
}

/// B11 review 2: an open while another holds the board's lock (as an
/// upgrade does) waits for it, past the usual 5 s, and never fails.
#[test]
fn an_open_waits_for_an_upgrade_holding_the_lock() {
    let (_dir, file, made) = schema_4_board();
    let holder = Connection::open(&file).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(6));
        holder.execute_batch("ROLLBACK").unwrap();
    });
    let opened = SqliteBoardOps::open(&file, identity());
    release.join().unwrap();
    assert!(opened.is_ok(), "{:?}", opened.err());
    assert_eq!(chains(&file), made);
}

#[test]
fn concurrent_opens_upgrade_a_schema_4_board_once() {
    let (_dir, file, made) = schema_4_board();
    let opens = 8;
    let barrier = Arc::new(Barrier::new(opens));
    let threads: Vec<_> = (0..opens)
        .map(|_| {
            let (file, barrier) = (file.clone(), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                SqliteBoardOps::open(&file, identity()).map(drop)
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), Ok(()), "no open fails spuriously");
    }
    assert_eq!(chains(&file), made);
    assert_eq!(copies(&file).len(), 1, "upgraded once, behind one copy");
}

#[test]
fn a_board_of_a_newer_build_says_to_upgrade_this_binary() {
    let (dir, ops) = board();
    wrote(&ops, 1, true, 0);
    drop(ops);
    let file = dir.path().join("board.sqlite3");
    Connection::open(&file)
        .unwrap()
        .execute_batch("PRAGMA user_version=6;")
        .unwrap();
    let Err(BoardStoreError::Clash(why)) = SqliteBoardOps::open(&file, identity()) else {
        panic!("refused");
    };
    assert!(why.contains("upgrade this binary"), "{why}");
    assert!(!why.contains("move the file aside"), "{why}");
}
