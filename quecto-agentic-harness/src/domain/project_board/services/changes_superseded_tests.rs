use super::tests::{host, merged, now, of};
use super::*;
use crate::domain::project_board::services::fixtures_tests::change;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::Review;

/// A task others took over is never done by its changes merging either.
#[test]
fn a_superseded_task_is_not_done_by_its_changes() {
    let by = vec![Slug::parse("next").unwrap()];
    let mut fields = of(Review).into_fields();
    fields.superseded_by = by.clone();
    let superseded = Task::new(fields).unwrap();
    let read = host(&[(7, "feat/b1", merged('a'))]);
    assert_eq!(
        complete(&superseded, &[change(1, Some(7))], &read, &[], &now()).map(|done| done.changes),
        Err(BoardRuleError::Superseded { by })
    );
}
