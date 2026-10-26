//! The upgrade's copy (epic #2705 B11 review 2 and 3): its room, its name,
//! when one beside the board is used again, and when it is removed.
use super::*;

/// A schema-4 board of three revisions in a temp dir: the dir and its path.
fn schema_4_board() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite3");
    let c = super::super::storage::open(&path).unwrap();
    c.execute_batch(
        "INSERT INTO revisions VALUES ('1:1','r','Ada','a@example.com',1,NULL,'one',1,NULL,NULL,NULL);
         INSERT INTO revisions VALUES ('1:2','r','Ada','a@example.com',2,1,'two',2,NULL,NULL,NULL);
         INSERT INTO revisions VALUES ('1:3','r','Ada','a@example.com',3,2,'three',3,NULL,NULL,NULL);
         ALTER TABLE revisions DROP COLUMN chain; PRAGMA user_version=4;",
    )
    .unwrap();
    (dir, path)
}

fn enough(_: &Path) -> Result<u64, String> {
    Ok(u64::MAX)
}

#[test]
fn free_bytes_are_read_and_a_missing_directory_is_said() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("none")).is_err());
}

#[test]
fn a_board_with_no_room_for_its_copy_is_refused_before_it_is_made() {
    let (_dir, board) = schema_4_board();
    let none = |_: &Path| Ok(0);
    let refused = kept_copy(&board, &none).unwrap_err();
    assert!(refused.contains("not enough free space"), "{refused}");
    assert!(copies(&board).is_empty(), "no copy is begun");
    // As large as all that is free: twice it is needed.
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("board.sqlite3");
    let free = free_bytes(dir.path()).unwrap();
    std::fs::File::create(&big).unwrap().set_len(free).unwrap();
    let refused = room(&big, &free_bytes).unwrap_err();
    assert!(refused.contains("not enough free space"), "{refused}");
}

#[test]
fn a_whole_copy_is_used_again_and_a_stale_or_corrupt_one_never() {
    let (_dir, board) = schema_4_board();
    let (first, made) = kept_copy(&board, &enough).unwrap();
    assert!(made);
    let (again, made) = kept_copy(&board, &enough).unwrap();
    assert_eq!((again, made), (first.clone(), false), "used again");
    // One row changes on the board: the copy is stale, never used again.
    let c = Connection::open(&board).unwrap();
    c.execute_batch("INSERT INTO op_results VALUES ('local','w','r','a')")
        .unwrap();
    drop(c);
    let (fresh, made) = kept_copy(&board, &enough).unwrap();
    assert!(made && fresh != first, "a new copy is made");
    assert!(whole(&fresh, &board) && !whole(&first, &board));
    // A file named like a copy that is no database is never used.
    let corrupt = board.with_file_name("board.sqlite3.schema4-00000000T000000Z");
    std::fs::write(&corrupt, b"not a board").unwrap();
    assert!(!whole(&corrupt, &board));
    assert_eq!(kept_copy(&board, &enough).unwrap(), (fresh, false));
}

#[test]
fn a_copy_is_removed_only_while_the_board_is_still_schema_4() {
    let (_dir, board) = schema_4_board();
    let (copy, _) = kept_copy(&board, &enough).unwrap();
    // The board upgraded after all (a commit that failed ambiguously): kept.
    drop(super::super::storage::open(&board).unwrap());
    let other = copies(&board);
    for copy in &other {
        std::fs::remove_file(copy).unwrap();
    }
    std::fs::write(&copy, b"kept").unwrap();
    remove_if_still_four(&board, &copy);
    assert!(copy.exists(), "the way back is kept");
    let (_dir, four) = schema_4_board();
    let (copy, _) = kept_copy(&four, &enough).unwrap();
    remove_if_still_four(&four, &copy);
    assert!(!copy.exists());
}

#[test]
fn a_reserved_copy_is_new_and_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.sqlite3");
    let first = reserved(&board).unwrap();
    let second = reserved(&board).unwrap();
    assert_ne!(first, second, "a name taken is never reused");
    let mode = std::fs::metadata(&first).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "private from the start");
}
