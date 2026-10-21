//! #2661: the tasks a task relates to hold back its claim (its
//! dependencies) and its archive (its children), naming them.
use super::*;
use crate::domain::project_board::services::fixtures_tests::task;
use crate::domain::project_board::value_objects::vocabulary::TaskKind;

fn slug(id: &str) -> Slug {
    Slug::parse(id).unwrap()
}

/// Task `id` in `status`, with `parent` and `depends_on`.
fn related(id: &str, status: TaskStatus, parent: Option<&str>, depends_on: &[&str]) -> Task {
    let mut fields = task(status).into_fields();
    fields.id = slug(id);
    fields.parent = parent.map(slug);
    fields.depends_on = depends_on.iter().map(|id| slug(id)).collect();
    fields.prs.clear();
    Task::new(fields).unwrap()
}

#[test]
fn a_claim_waits_for_every_dependency_done_or_archived() {
    let a = related("a", Ready, None, &["x", "y", "gone"]);
    let board = |x, y| vec![related("x", x, None, &[]), related("y", y, None, &[])];
    assert_eq!(
        relations_allow(&a, Claimed, &board(Ready, Done)),
        Err(BoardRuleError::DependencyNotDone {
            undone: vec![slug("x")],
            missing: vec![slug("gone")],
        }),
        "one not done, and one not on the board"
    );
    let b = related("b", Ready, None, &["x", "y"]);
    for (x, y) in [(Done, Archived), (Archived, Archived), (Done, Done)] {
        assert_eq!(
            relations_allow(&b, Claimed, &board(x, y)),
            Ok(()),
            "{x:?} {y:?}"
        );
    }
    for held in [InProgress, Review, Blocked, Claimed, Draft] {
        assert_eq!(
            relations_allow(&b, Claimed, &board(held, Done)),
            Err(BoardRuleError::DependencyNotDone {
                undone: vec![slug("x")],
                missing: vec![],
            }),
            "{held:?}"
        );
    }
    // Only a claim waits on dependencies.
    for to in [Draft, Ready, InProgress, Review, Blocked, Done, Archived] {
        assert_eq!(
            relations_allow(&b, to, &board(Ready, Ready)),
            Ok(()),
            "{to:?}"
        );
    }
}

#[test]
fn an_archive_waits_for_every_child_free_of_work() {
    let mut fields = related("epic", Draft, None, &[]).into_fields();
    fields.kind = TaskKind::Epic;
    let epic = Task::new(fields).unwrap();
    let child = |id, status| related(id, status, Some("epic"), &[]);
    let board = vec![
        child("c1", InProgress),
        child("c2", Ready),
        child("c3", Review),
        child("c4", Done),
        related("other", InProgress, None, &[]),
    ];
    assert_eq!(
        relations_allow(&epic, Archived, &board),
        Err(BoardRuleError::ChildrenActive(vec![slug("c1"), slug("c3")]))
    );
    for held in [Claimed, Blocked] {
        assert_eq!(
            relations_allow(&epic, Archived, &[child("c", held)]),
            Err(BoardRuleError::ChildrenActive(vec![slug("c")])),
            "{held:?}"
        );
    }
    let idle = [
        child("c1", Draft),
        child("c2", Ready),
        child("c3", Done),
        child("c4", Archived),
    ];
    assert_eq!(relations_allow(&epic, Archived, &idle), Ok(()));
    // Only an archive waits on children.
    assert_eq!(relations_allow(&epic, Ready, &board), Ok(()));
}

/// #2661 review round 1: the domain's own move asks the relations too.
#[test]
fn a_move_asks_the_tasks_it_relates_to() {
    let a = related("a", Ready, None, &["x"]);
    let board = [related("x", Ready, None, &[])];
    let actor = crate::domain::project_board::services::fixtures_tests::person("Bob");
    let now = crate::domain::project_board::services::fixtures_tests::at("10:00:00");
    assert_eq!(
        move_task(&a, Claimed, &actor, (&[], &[]), &board, &now),
        Err(BoardRuleError::DependencyNotDone {
            undone: vec![slug("x")],
            missing: vec![],
        })
    );
    let done = [related("x", Done, None, &[])];
    assert!(move_task(&a, Claimed, &actor, (&[], &[]), &done, &now).is_ok());
}
