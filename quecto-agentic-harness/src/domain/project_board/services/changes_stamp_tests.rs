//! #2662: a change's stamps never run backwards on its task.
use super::tests::{head, host, now, of, on};
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, change, last_written, person};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::InProgress;

/// #2662: linking or unlinking a change never stamps it before its task's
/// last change, so stamps never run backwards on one task.
#[test]
fn a_change_is_never_stamped_before_its_tasks_last_change() {
    let task = last_written(of(InProgress), &at("10:00:05"));
    let ada = person("Ada");
    let read = host(&[
        (7, "feat/b1", PrState::Open),
        (8, "feat/second", PrState::Open),
    ]);
    let own = [change(1, Some(7))];
    let linked = link(
        &task,
        on(&own),
        head("feat/second", Some(8)),
        &read,
        &ada,
        &now(),
    )
    .unwrap();
    assert_eq!(
        (&linked.fields().created, &linked.fields().updated),
        (&at("10:00:05"), &at("10:00:05"))
    );
    let gone = unlink(&task, &own, 1, &read, &ada, &now()).unwrap();
    assert_eq!(gone.fields().updated, at("10:00:05"));
}
