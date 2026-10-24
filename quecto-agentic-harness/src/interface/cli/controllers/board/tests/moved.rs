//! `quecto board` on a board moved to a board server (#2705 B13): a read
//! and a write each answer `board_moved`, naming the URLs, and exit 1.
use super::*;
use crate::application::project_board::ports::{BoardStoreError, ProjectBoardStore};
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use crate::domain::project_board::value_objects::telemetry::BoardId;

fn moved_board(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let id = BoardId::of_hex("0123456789abcdef").unwrap();
    let urls = vec![
        "https://board.example:7443".into(),
        "http://100.64.0.2:7443".into(),
    ];
    store.fail_every_call(BoardStoreError::Moved(BoardMoved::new(id, urls).unwrap()));
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

#[test]
fn a_moved_board_answers_board_moved_on_the_command_line() {
    for line in [
        &["show", "--task", "a"][..],
        &["move", "--task", "a", "--to", "claimed"][..],
    ] {
        let (code, answer, _) = cli(Some(moved_board), line);
        assert_eq!(code, 1, "{answer}");
        assert_eq!(answer["error"]["code"], json!("board_moved"), "{answer}");
        assert_eq!(answer["error"]["board"], json!("0123456789abcdef"));
        assert_eq!(
            answer["error"]["urls"],
            json!(["https://board.example:7443", "http://100.64.0.2:7443"])
        );
    }
}
