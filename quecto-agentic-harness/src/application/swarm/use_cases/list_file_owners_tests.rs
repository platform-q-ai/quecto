use serde_json::{Value, json};

use super::ListFileOwners;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, StoredFile, member_row, running_board,
};
use crate::application::swarm::dto::ListFileOwnersRequest;
use crate::domain::swarm::{BoardError, RefusalKind};

fn page(actor: &str, offset: Value, limit: Value) -> ListFileOwnersRequest {
    ListFileOwnersRequest {
        actor: actor.to_owned(),
        offset,
        limit,
    }
}

fn board() -> std::sync::Arc<MemoryBoard> {
    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    state.files = ["c", "a", "b"]
        .map(|path| StoredFile {
            path: path.to_owned(),
            task: 1,
            owner: "worker".to_owned(),
            claim: "claim".to_owned(),
            token: "token".to_owned(),
        })
        .to_vec();
    MemoryBoard::with(state)
}

/// A page of the reservations by path, readable by a dead member too.
#[test]
fn a_page_lists_reservations_by_path() {
    let board = board();
    let service = ListFileOwners::new(board, SteppingClock::fixed(50.0));
    let rows = service.execute(page("gone", json!(1), json!(50))).unwrap();
    let paths: Vec<Value> = rows
        .into_iter()
        .map(|row| row.into_value()["path"].clone())
        .collect();
    assert_eq!(paths, [json!("b"), json!("c")]);
}

/// Only an integer offset of at least 0 and an integer limit from 1 to
/// 100 page (a boolean or a float is not an `int`), refused before the
/// gate.
#[test]
fn only_an_integer_page_is_read() {
    let board = board();
    let service = ListFileOwners::new(board.clone(), SteppingClock::fixed(50.0));
    for (offset, limit) in [
        (json!(-1), json!(50)),
        (json!(0), json!(0)),
        (json!(0), json!(101)),
        (json!(true), json!(50)),
        (json!(0), json!(true)),
        (json!(0.0), json!(50)),
        (json!(0), json!(5.0)),
        (json!("0"), json!(50)),
        (json!(null), json!(50)),
    ] {
        assert_eq!(
            service
                .execute(page("parent", offset.clone(), limit.clone()))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "file page requires nonnegative offset and limit 1 through 100"
            ),
            "{offset} {limit}"
        );
    }
    assert!(board.transactions().is_empty());
    for (offset, limit) in [(json!(0), json!(1)), (json!(9), json!(100))] {
        service.execute(page("parent", offset, limit)).unwrap();
    }
}
