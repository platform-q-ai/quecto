use serde_json::json;

use super::woken;
use crate::application::swarm::board_test_support::{
    MemoryBoard, member_row, running_board, stored_member,
};
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::NotificationEvent;

/// An amendment wakes every live member but the actor, each answered as
/// its whole row in id order; a row whose id is not text is no member the
/// policy can name.
#[test]
fn the_woken_are_member_rows_in_id_order() {
    let mut state = running_board(100.0);
    state.members.push(member_row("zed", "live"));
    state.members.push(member_row("amy", "live"));
    state.members.push(member_row("dead", "dead"));
    state.members.push(stored_member(
        "x",
        json!("r"),
        "live",
        [json!(null), json!(null), json!(null), json!(null)],
    ));
    state.members.last_mut().unwrap().columns[0].1 = json!(7);
    let run = state.run.as_ref().unwrap().record.clone();
    let board = MemoryBoard::with(state);
    let amended = [NotificationEvent {
        action: "amended".to_owned(),
        detail: json!({"reason": "r"}),
        actor: Some("parent".to_owned()),
    }];
    let mut found = Vec::new();
    board
        .atomic(false, &mut |transaction| {
            found = woken(transaction, &run, "parent", &amended)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        found,
        [member_row("amy", "live"), member_row("zed", "live")]
    );
}
