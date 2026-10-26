//! The upgrade's room and copy name (epic #2705 B11 review 2).
use super::*;

#[test]
fn free_bytes_are_read_and_a_missing_directory_is_said() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("none")).is_err());
}

#[test]
fn a_board_with_no_room_for_its_copy_is_refused_before_it_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let board = dir.path().join("board.sqlite3");
    // A board larger than half of what is free has no room for its copy.
    let free = free_bytes(dir.path()).unwrap();
    let file = std::fs::File::create(&board).unwrap();
    file.set_len(free / 2 + 1).unwrap();
    let refused = room(&board).unwrap_err();
    assert!(refused.contains("not enough free space"), "{refused}");
    std::fs::write(&board, b"small").unwrap();
    assert_eq!(room(&board), Ok(()));
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
