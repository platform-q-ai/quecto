use super::project_board_store::task;
use quecto::application::project_board::ports::{BoardArtifact, BoardWrite, ProjectBoardStore};
use quecto::domain::project_board::entities::claim::Identity;
use quecto::infrastructure::persistence::{
    project_board::artifacts::file_of, sqlite::project_board::SqliteProjectBoardStore,
};

#[test]
fn stored_canonical_blob_is_byte_exact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite");
    let store = SqliteProjectBoardStore::open(
        &path,
        Identity {
            name: "Alice".into(),
            email: "alice@example.com".into(),
        },
    )
    .unwrap();
    store.bootstrap().unwrap();
    let artifact = BoardArtifact::Task(task("a", "Canonical bytes"));
    let expected = file_of(&artifact).unwrap();
    assert!(matches!(
        store.write(&store.read().unwrap().head, &[artifact], "board: bytes"),
        Ok(BoardWrite::Written(_))
    ));
    let raw = rusqlite::Connection::open(&path).unwrap();
    let (kind, bytes): (String, Vec<u8>) = raw
        .query_row(
            "SELECT typeof(bytes),bytes FROM files WHERE path='tasks/a.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "blob");
    assert_eq!(bytes, expected);
}
