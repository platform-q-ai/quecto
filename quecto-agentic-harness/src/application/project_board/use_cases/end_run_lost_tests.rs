//! #2657: a run ended as lost recorded `items: []` and no
//! `items_recorded`, while a stopped one recorded every item as todo; a
//! worker had finished item 1 before the container went, and none of it
//! reached the board. A lost run records what is known, as a stop does:
//! the board its swarm left when one was kept, else every plan item as
//! todo.
use super::*;

#[test]
fn a_lost_run_records_the_board_its_swarm_left() {
    let (store, _) = stranded();
    let states = [
        SwarmItemState::Completed,
        SwarmItemState::Claimed,
        SwarmItemState::Ready,
    ];
    let ended = ender_keeping(&store, Some(swarm(SwarmEnd::Cancelled, states)))
        .execute(&ending(RunEndOutcome::Lost))
        .unwrap();
    assert_eq!(
        ended.report,
        RunReport::Kept,
        "recorded from the kept board"
    );
    let fields = ended.run.fields();
    assert_eq!(
        (fields.outcome, fields.cost),
        (Some(RunOutcome::Lost), Some(52_000))
    );
    let states: Vec<ItemState> = fields.items.iter().map(|item| item.state).collect();
    assert_eq!(states, [ItemState::Done, ItemState::Doing, ItemState::Todo]);
    assert_eq!(fields.items[0].owner.as_deref(), Some("worker-1"));
    assert_eq!(fields.items[0].evidence, ["docs/api.md at abc123"]);
    assert_eq!(ended.run, run_1(&store));
}

#[test]
fn a_lost_run_with_no_board_kept_records_every_item_as_todo() {
    let (store, _) = stranded();
    let ended = ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Lost))
        .unwrap();
    assert_eq!(ended.report, RunReport::Unreported);
    let fields = ended.run.fields();
    assert_eq!(
        (fields.outcome, fields.cost),
        (Some(RunOutcome::Lost), None)
    );
    let ids: Vec<&str> = fields.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["api", "store", "docs"], "every plan item");
    assert!(
        fields
            .items
            .iter()
            .all(|item| item.state == ItemState::Todo && item.evidence.is_empty())
    );
}
