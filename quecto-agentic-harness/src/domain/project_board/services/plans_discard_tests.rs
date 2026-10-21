//! #2481 M4c: a discard is the claim holder's decision on a draft, pinned
//! as an edit is; an edit records its editor; and #2662: a plan's change
//! never stamps before its task or another plan of it.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, plan, task};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::{self, Ready};

/// `now`, with no outside bound.
fn when(now: &Timestamp) -> Moment {
    Moment {
        now: now.clone(),
        latest: None,
    }
}

/// #2481 M4c: a discard is pinned as an edit is: to the draft as the
/// caller read it, and never to a draft a clock running ahead stamped.
#[test]
fn a_discard_pinned_to_an_older_read_or_a_plan_from_ahead_is_refused() {
    let draft = plan(2, PlanState::Draft);
    let bob = person("Bob");
    let now = when(&at("10:00:00"));
    assert_eq!(
        discard(
            DraftOf {
                task: &task(Ready),
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            Some(&at("08:00:00")),
            &bob,
            &now
        ),
        Err(BoardRuleError::PlanChanged {
            version: 2,
            updated: at("08:30:00"),
        })
    );
    let read = discard(
        DraftOf {
            task: &task(Ready),
            versions: std::slice::from_ref(&draft),
            plan: &draft,
        },
        Some(&at("08:30:00")),
        &bob,
        &now,
    )
    .unwrap();
    assert_eq!(read.fields().state, PlanState::Discarded);
    let bounded = Moment {
        now: at("08:00:00"),
        latest: Some(at("08:00:00")),
    };
    assert_eq!(
        discard(
            DraftOf {
                task: &task(Ready),
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            None,
            &bob,
            &bounded
        ),
        Err(BoardRuleError::PlanAhead {
            version: 2,
            updated: at("08:30:00"),
        })
    );
}

/// #2662: a plan's change never stamps before its task's last change,
/// so stamps never run backwards on one task: a proposal, an edit and a
/// discard from a clock that reads before the task's `updated` (a second
/// past the wall clock after quick changes) are stamped no earlier.
#[test]
fn a_plans_change_never_stamps_before_its_tasks_last_change() {
    let task = crate::domain::project_board::services::fixtures_tests::last_written(
        task(Ready),
        &at("10:00:01"),
    );
    let draft = plan(2, PlanState::Draft);
    let bob = person("Bob");
    let now = when(&at("10:00:00"));
    let content = PlanContent {
        approach: "Build it.".into(),
        ..PlanContent::default()
    };
    let versions = [plan(1, PlanState::Accepted), draft.clone()];
    let proposed = propose_at(
        &task,
        &versions,
        content.clone(),
        &bob,
        &when(&now.now.clone()),
    )
    .unwrap();
    let edited = edit_draft(
        DraftOf {
            task: &task,
            versions: std::slice::from_ref(&draft),
            plan: &draft,
        },
        content,
        None,
        &bob,
        &now,
    )
    .unwrap();
    let discarded = discard(
        DraftOf {
            task: &task,
            versions: std::slice::from_ref(&draft),
            plan: &draft,
        },
        None,
        &bob,
        &now,
    )
    .unwrap();
    for (change, plan) in [
        ("propose", &proposed),
        ("edit", &edited),
        ("discard", &discarded),
    ] {
        assert!(
            plan.fields().updated >= task.fields().updated,
            "{change}: {:?} before the task's {:?}",
            plan.fields().updated,
            task.fields().updated
        );
    }
    assert_eq!(proposed.fields().created, at("10:00:01"));
}

/// `plan` as last changed at `updated` (by quick edits, say).
fn stamped(plan: Plan, updated: &str) -> Plan {
    let mut fields = plan.into_fields();
    fields.updated = at(updated);
    Plan::new(fields).unwrap()
}

/// #2662 review round 1: a plan's change is never stamped before any of
/// the task's plans either: a proposal after quick edits of a draft, and
/// an edit or a discard of one draft after another was changed later.
#[test]
fn a_plans_change_never_stamps_before_another_plan_of_its_task() {
    let task = task(Ready);
    let bob = person("Bob");
    let now = when(&at("10:00:00"));
    let v2 = stamped(plan(2, PlanState::Draft), "10:00:03");
    let versions = [plan(1, PlanState::Accepted), v2.clone()];
    let proposed = propose_at(
        &task,
        &versions,
        PlanContent::default(),
        &bob,
        &when(&now.now.clone()),
    )
    .unwrap();
    assert_eq!(
        (&proposed.fields().created, &proposed.fields().updated),
        (&at("10:00:03"), &at("10:00:03")),
        "a proposal after quick edits"
    );
    let v3 = stamped(plan(3, PlanState::Draft), "10:00:05");
    let early = stamped(plan(2, PlanState::Draft), "09:30:00");
    let versions = [plan(1, PlanState::Accepted), early.clone(), v3];
    let content = PlanContent {
        approach: "Build it.".into(),
        ..PlanContent::default()
    };
    let edited = edit_draft(
        DraftOf {
            task: &task,
            versions: &versions,
            plan: &early,
        },
        content,
        None,
        &bob,
        &now,
    )
    .unwrap();
    assert_eq!(edited.fields().updated, at("10:00:05"), "an edit");
    let discarded = discard(
        DraftOf {
            task: &task,
            versions: &versions,
            plan: &early,
        },
        None,
        &bob,
        &now,
    )
    .unwrap();
    assert_eq!(discarded.fields().updated, at("10:00:05"), "a discard");
}

#[test]
fn a_discarded_draft_is_kept_and_only_a_draft_is_discarded() {
    let discarded = discard(
        DraftOf {
            task: &task(Ready),
            versions: std::slice::from_ref(&plan(2, PlanState::Draft)),
            plan: &plan(2, PlanState::Draft),
        },
        None,
        &person("Bob"),
        &when(&at("10:00:00")),
    )
    .unwrap();
    assert_eq!(discarded.fields().state, PlanState::Discarded);
    assert_eq!(
        discarded.fields().items,
        plan(2, PlanState::Draft).fields().items
    );
    assert_eq!(
        discard(
            DraftOf {
                task: &task(Ready),
                versions: std::slice::from_ref(&discarded),
                plan: &discarded,
            },
            None,
            &person("Bob"),
            &when(&at("10:00:00"))
        ),
        Err(BoardRuleError::PlanNotDraft(PlanState::Discarded))
    );
}

/// Drafts are anyone's: an edit records who made it, and keeps who
/// proposed the plan.
#[test]
fn an_edit_records_its_editor_and_keeps_the_proposer() {
    let changed = PlanContent {
        approach: "Build it.".into(),
        ..PlanContent::default()
    };
    let edited = edit_draft(
        DraftOf {
            task: &task(TaskStatus::InProgress),
            versions: std::slice::from_ref(&plan(2, PlanState::Draft)),
            plan: &plan(2, PlanState::Draft),
        },
        changed,
        None,
        &person("Bob"),
        &when(&at("10:00:00")),
    )
    .unwrap();
    assert_eq!(
        (&edited.fields().proposed_by, &edited.fields().edited_by),
        (&person("Ada"), &Some(person("Bob")))
    );
}

/// #2662 review round 2: an acceptance is stamped no earlier than any of
/// the task's plans, a sibling draft changed later included.
#[test]
fn an_acceptance_never_stamps_before_another_plan_of_its_task() {
    let task = task(Ready);
    let chosen = stamped(plan(2, PlanState::Draft), "09:30:00");
    let sibling = stamped(plan(3, PlanState::Draft), "10:00:07");
    let versions = [plan(1, PlanState::Accepted), chosen, sibling];
    let board = TaskPlans {
        task: &task,
        versions: &versions,
        runs: &[],
    };
    let accepted = accept(&board, 2, None, &person("Bob"), &when(&at("10:00:00"))).unwrap();
    assert_eq!(accepted.plan.fields().updated, at("10:00:07"));
    assert_eq!(accepted.task.fields().updated, at("10:00:07"));
}

/// #2662 review round 2: a sibling stamped past the outside bound (a hand
/// edit, a clock that ran ahead) never drags a new stamp after it.
#[test]
fn a_sibling_past_the_bound_is_left_out_of_the_floor() {
    let task = task(Ready);
    let future = stamped(plan(2, PlanState::Draft), "23:00:00");
    let versions = [plan(1, PlanState::Accepted), future];
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:01:00")),
    };
    let proposed = propose_at(
        &task,
        &versions,
        PlanContent::default(),
        &person("Bob"),
        &bounded,
    )
    .unwrap();
    assert_eq!(proposed.fields().created, at("10:00:00"));
}

/// #2662 review round 3: an acceptance leaves a sibling plan stamped past
/// the outside bound out of its floor too.
#[test]
fn an_acceptance_leaves_a_sibling_past_the_bound_out_of_its_floor() {
    let task = task(Ready);
    let chosen = stamped(plan(2, PlanState::Draft), "09:30:00");
    let future = stamped(plan(3, PlanState::Draft), "23:00:00");
    let versions = [plan(1, PlanState::Accepted), chosen, future];
    let board = TaskPlans {
        task: &task,
        versions: &versions,
        runs: &[],
    };
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:01:00")),
    };
    let accepted = accept(&board, 2, None, &person("Bob"), &bounded).unwrap();
    assert_eq!(accepted.plan.fields().updated, at("10:00:00"));
}
