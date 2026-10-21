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

/// #2662 review round 1: a change is never stamped before another change
/// of its task either: a link after an unlink a fast clock stamped, and
/// an unlink of one change after another changed later.
#[test]
fn a_change_is_never_stamped_before_another_change_of_its_task() {
    let task = of(InProgress);
    let ada = person("Ada");
    let read = host(&[
        (7, "feat/b1", PrState::Open),
        (8, "feat/second", PrState::Open),
    ]);
    let mut fields = change(1, Some(7)).into_fields();
    fields.unlinked = Some((ada.clone(), at("10:00:05")));
    fields.updated = at("10:00:05");
    let gone = Change::new(fields).unwrap();
    let linked = link(
        &task,
        on(std::slice::from_ref(&gone)),
        head("feat/second", Some(8)),
        &read,
        &ada,
        &now(),
    )
    .unwrap();
    assert_eq!(
        (&linked.fields().created, &linked.fields().updated),
        (&at("10:00:05"), &at("10:00:05")),
        "a link after an unlink"
    );
    let own = [gone, change(2, Some(8))];
    let unlinked = unlink(&task, &own, 2, &read, &ada, &now()).unwrap();
    assert_eq!(unlinked.fields().updated, at("10:00:05"), "an unlink");
}

/// #2662 review round 3: a sibling change stamped past the outside bound
/// is refused before anything is built on it (`change_ahead`), so no
/// link's or unlink's stamp is dragged after it.
#[test]
fn a_sibling_change_past_the_bound_is_refused_before_the_floor() {
    let task = of(InProgress);
    let ada = person("Ada");
    let read = host(&[
        (7, "feat/b1", PrState::Open),
        (8, "feat/second", PrState::Open),
    ]);
    let mut fields = change(1, Some(7)).into_fields();
    fields.updated = at("23:00:00");
    let future = Change::new(fields).unwrap();
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:01:00")),
    };
    let ahead = BoardRuleError::ChangeAhead {
        change: 1,
        updated: at("23:00:00"),
    };
    let own = [future.clone()];
    let linked = link(
        &task,
        on(&own),
        head("feat/second", Some(8)),
        &read,
        &ada,
        &bounded,
    );
    assert_eq!(linked, Err(ahead.clone()), "a link");
    let both = [future, change(2, Some(8))];
    assert_eq!(
        unlink(&task, &both, 2, &read, &ada, &bounded),
        Err(ahead),
        "an unlink"
    );
}
