use super::ProjectBoardError;
use crate::application::project_board::ports::{BoardClockError, BoardStoreError};

/// A store's failure says which kind it is, not only the store's words.
#[test]
fn a_store_failure_names_its_kind() {
    let shown = |error: BoardStoreError| ProjectBoardError::Store(error).to_string();
    assert_eq!(
        shown(BoardStoreError::Clash("refs/heads/quecto exists".into())),
        "the board clashes with the repository: refs/heads/quecto exists"
    );
    assert_eq!(
        shown(BoardStoreError::InvalidWrite(
            "a write holds each task once".into()
        )),
        "the board refused an invalid write: a write holds each task once"
    );
    assert_eq!(
        shown(BoardStoreError::Unavailable("the remote hung up".into())),
        "the board store is unavailable: the remote hung up"
    );
    assert_eq!(
        shown(BoardStoreError::NoBoard),
        "the project has no board yet"
    );
}

#[test]
fn a_clock_that_cannot_tell_the_time_is_reported() {
    let error: ProjectBoardError =
        BoardClockError::OutOfRange("1969-12-31T23:59:59Z".into()).into();
    assert_eq!(
        error.to_string(),
        "this clock reads 1969-12-31T23:59:59Z, a time the board cannot spell; fix the clock"
    );
}
