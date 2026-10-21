//! #2656: a renewal before halfway through its claim writes nothing.
use super::*;

/// #2656: a renewal while the claim still has more than half its two
/// hours left writes nothing and answers the task as it is; from halfway
/// on, it renews.
#[test]
fn a_renewal_before_halfway_writes_nothing() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::InProgress));
    let a = slug("a");
    let writes = store.writes();
    let early = change(&store, "Ada", "09:30:00")
        .execute(&a, &BoardOp::Renew)
        .unwrap();
    assert_eq!(store.writes(), writes, "nothing written before halfway");
    let mut facts = crate::application::project_board::telemetry::OpFacts::default();
    change(&store, "Ada", "09:31:00")
        .execute_noted(&a, &BoardOp::Renew, Some(&mut facts))
        .unwrap();
    assert_eq!(facts.attempts, Some(0), "no attempt made");
    assert!(facts.claim.is_some_and(|claim| claim.holder_is_writer));
    assert_eq!(
        early.fields().claim.as_ref().unwrap().expires,
        at("11:00:00")
    );
    let halfway = change(&store, "Ada", "10:00:00")
        .execute(&a, &BoardOp::Renew)
        .unwrap();
    assert_eq!(
        halfway.fields().claim.as_ref().unwrap().expires,
        at("12:00:00")
    );
    // Another's renewal is refused as ever, before halfway too.
    assert_eq!(
        change(&store, "Bob", "10:01:00").execute(&a, &BoardOp::Renew),
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolder))
    );
}
