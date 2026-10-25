//! #2741 review round 2: a served board file is bound to the server's
//! board id, at start and on its first write; another id is refused.
use super::*;

const BOARD: &str = "0123456789abcdef";

fn identity() -> Identity {
    Identity {
        name: "Server".into(),
        email: "server@example.com".into(),
    }
}

fn bound(path: &Path) -> Option<String> {
    let c = Connection::open(path).unwrap();
    c.query_row("SELECT board_id FROM history_meta WHERE id=1", [], |r| {
        r.get(0)
    })
    .optional()
    .unwrap()
    .flatten()
}

#[test]
fn a_served_board_is_bound_on_its_first_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite3");
    let ops = SqliteBoardOps::open(&path, identity()).unwrap();
    assert_eq!(
        ops.bind(BOARD),
        Ok(BoardBinding::Bound),
        "nothing to bind yet"
    );
    assert_eq!(bound(&path), None);
    let write = OpWrite {
        write_id: WriteId::parse("00000000-0000-4000-8000-000000000000").unwrap(),
        request: "cli\n{}".into(),
        committer: identity(),
        principal: "local".into(),
    };
    let made = ops.transact(&write, &mut |store| {
        store.bootstrap().map(|_| "made".to_string())
    });
    assert_eq!(made, Ok(Transacted::Answered("made".into())));
    assert_eq!(bound(&path).as_deref(), Some(BOARD));
}

#[test]
fn an_unbound_board_is_bound_at_start_and_another_id_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite3");
    SqliteProjectBoardStore::open(&path, identity())
        .unwrap()
        .bootstrap()
        .unwrap();
    assert_eq!(bound(&path), None, "a board made off the server is unbound");
    let ops = SqliteBoardOps::open(&path, identity()).unwrap();
    assert_eq!(ops.bind(BOARD), Ok(BoardBinding::Bound));
    assert_eq!(bound(&path).as_deref(), Some(BOARD));
    let other = SqliteBoardOps::open(&path, identity()).unwrap();
    assert_eq!(
        other.bind("fedcba9876543210"),
        Ok(BoardBinding::Other(BOARD.into()))
    );
    assert_eq!(bound(&path).as_deref(), Some(BOARD), "left as bound");
}
