//! A board moved to a board server (#2705 B13) answers `board_moved`,
//! naming its id and the URLs that serve it; a malformed `MOVED` answers
//! `board_moved_malformed` with its reason.
use super::*;
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use serde_json::json;

pub(super) fn moved() -> BoardMoved {
    let id = BoardId::of_hex("0123456789abcdef").unwrap();
    let urls = vec![
        "https://board.example:7443".into(),
        "http://100.64.0.2:7443".into(),
    ];
    BoardMoved::new(id, urls).unwrap()
}

pub(super) fn malformed() -> BoardStoreError {
    BoardStoreError::MovedMalformed("format/MOVED names no url".into())
}

#[test]
fn a_moved_board_answers_board_moved_with_its_id_and_urls() {
    let error = ProjectBoardError::Store(BoardStoreError::Moved(moved()));
    let answer = board_refused_with("list", &error, None);
    assert_eq!(answer["ok"], json!(false));
    assert_eq!(answer["error"]["code"], json!("board_moved"));
    assert_eq!(answer["error"]["board"], json!("0123456789abcdef"));
    assert_eq!(
        answer["error"]["urls"],
        json!(["https://board.example:7443", "http://100.64.0.2:7443"])
    );
    let message = answer["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with(
            "this board moved to a board server at https://board.example:7443, \
             http://100.64.0.2:7443"
        ),
        "{message}"
    );
    assert!(message.contains("only through that server"), "{message}");
    assert_eq!(error_code(&error), ErrorCode::BoardMoved);
}

#[test]
fn a_malformed_moved_file_answers_its_own_code_and_reason() {
    let answer = board_refused_with("create", &ProjectBoardError::Store(malformed()), None);
    assert_eq!(answer["error"]["code"], json!("board_moved_malformed"));
    assert_eq!(
        answer["error"]["reason"],
        json!("format/MOVED names no url")
    );
}
