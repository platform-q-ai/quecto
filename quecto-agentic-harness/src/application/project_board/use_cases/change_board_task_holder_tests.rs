//! Holder-only ops answer the holder rule first, and the same way (#2683,
//! #2686): another's expired claim is refused with `expired` and the
//! take_over way on, for a release and a renewal as for a link; and a link
//! by someone who may not link is refused as such, before any file set
//! aside elsewhere on the board is named.
use super::tests::{change, held_by_ada, seed};
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::use_cases::fixtures_tests::{
    board, fields, set_aside_task, slug,
};
use crate::domain::project_board::services::transitions::MoveReason;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// #2683: Bob renewing or releasing Ada's expired claim is told to take
/// the task over first, as accept, discard and link_pr tell him; nothing
/// is written.
#[test]
fn another_renewing_or_releasing_an_expired_claim_is_told_to_take_it_over() {
    for from in [TaskStatus::Claimed, TaskStatus::InProgress] {
        let store = board();
        seed(&store, held_by_ada(from));
        let writes = store.writes();
        // A task in progress is released by its run's end (#2705 T5).
        let release = (from == TaskStatus::Claimed)
            .then_some(BoardOp::Move(TaskStatus::Ready, MoveReason::Absent));
        for op in std::iter::once(BoardOp::Renew).chain(release) {
            let refused = change(&store, "Bob", "14:00:00").execute(&slug("a"), &op);
            assert_eq!(
                refused,
                Err(ProjectBoardError::Rule(BoardRuleError::NotHolderExpired)),
                "{from:?} {op:?}"
            );
        }
        assert_eq!(store.writes(), writes, "{from:?}");
    }
}

/// #2686: with an unrelated task file set aside, a link by someone who may
/// not link answers the holder rule, never the set-aside file; the holder
/// of a live claim is still refused over the file, which may link the same
/// pull request.
#[test]
fn a_link_answers_the_holder_rule_before_any_file_set_aside() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Claimed));
    seed(&store, fields("broken"));
    store.quarantine("broken", "malformed");
    let writes = store.writes();
    let link =
        |who: &str, time: &str| change(&store, who, time).execute(&slug("a"), &BoardOp::AddPr(7));
    assert_eq!(
        link("Bob", "10:00:00"),
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolder))
    );
    assert_eq!(
        link("Bob", "14:00:00"),
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolderExpired))
    );
    assert_eq!(
        link("Ada", "14:00:00"),
        Err(ProjectBoardError::Rule(BoardRuleError::ClaimExpired))
    );
    assert_eq!(link("Ada", "10:00:00"), Err(set_aside_task("broken")));
    assert_eq!(store.writes(), writes);
}
